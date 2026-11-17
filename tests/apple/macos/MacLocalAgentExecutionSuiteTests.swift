// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS local agent execution, credential gate, process session, resume authorization/frame, prompt boundary, model route, and terminal contract suite.

import Darwin
import Foundation
import Testing
import XCTest
@testable import Nola

@MainActor
final class MacLocalAgentCredentialGateTests: XCTestCase {
    nonisolated final class Credentials: @unchecked Sendable {
        private let lock = NSLock()
        private var values: [String: String] = ["endpoint:endpoint:original": "fixture-key"]
        func read(_ account: String) -> String? { lock.lock(); defer { lock.unlock() }; return values[account] }
        func set(_ account: String, _ value: String?) { lock.lock(); defer { lock.unlock() }; values[account] = value }
    }

    @MainActor struct Fixture {
        let root: URL
        let port: MacApplicationPort
        let credentials: Credentials
        let store: MacAppleChatConfigurationStore

        init() throws {
            root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-credential-gate-\(UUID())")
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            port = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
            credentials = Credentials()
            let reader = credentials
            store = MacAppleChatConfigurationStore(applicationPort: port, credentialReader: { reader.read($0) })
            try save()
        }

        func save(account: String = "endpoint:endpoint:original", enabled: Bool = true,
                  endpointEnabled: Bool = true, deleted: Bool = false,
                  endpoint: String = "https://fixture.invalid/v1", model: String = "upstream-model",
                  wireProtocol: String = "chat_completions") throws {
            let source: [String: Any] = ["schemaVersion": 1, "endpointID": "endpoint",
                "endpointName": "Fixture", "baseURL": endpoint,
                "endpointEnabled": endpointEnabled, "endpointOrder": 0, "modelEnabled": true,
                "modelOrder": 0, "priority": 100, "capabilities": ["chat"],
                "wireProtocol": wireProtocol, "credentialAccount": account]
            let current = try port.listModelProfiles().first { $0.id == "profile" }
            try port.upsertModelProfile(MacCanonicalModelProfile(id: "profile", displayName: "Fixture",
                provider: "openai-compatible", modelName: model, runtime: "remote",
                sourceJSON: String(decoding: try JSONSerialization.data(withJSONObject: source), as: UTF8.self),
                parametersJSON: "{}", selected: false, enabled: enabled, revision: current?.revision ?? 0,
                createdAtMs: 1, updatedAtMs: 2, deletedAtMs: deleted ? 2 : nil))
        }

        func request(profileID: String? = "profile", key: String = "fixture-key",
                     fallbacks: [MacRemoteModelFallback] = []) throws -> MacLocalAgentRunRequest {
            try MacLocalAgentRunRequest(conversationID: "conversation", prompt: "fixture",
                projectURL: root, attachmentURLs: [], modelID: "upstream-model", modelProfileID: profileID,
                endpoint: "https://fixture.invalid/v1", apiKey: key, modelTestMode: true, modelFallbacks: fallbacks,
                memoryEnabled: false, databaseURLOverride: root.appendingPathComponent("fixture.sqlite"),
                applicationPort: port)
        }

        func close() { try? FileManager.default.removeItem(at: root) }
    }

    func testRevokedChangedDisabledAndReboundCredentialsNeverDispatch() throws {
        for change in ["deleted-key", "changed-key", "disabled-profile", "disabled-endpoint", "changed-account", "deleted-profile", "changed-endpoint", "changed-model", "changed-protocol"] {
            let fixture = try Fixture()
            defer { fixture.close() }
            let selection = try MacLocalAgentCredentialGate.capture(fixture.request(), store: fixture.store)
            switch change {
            case "deleted-key": fixture.credentials.set("endpoint:endpoint:original", nil)
            case "changed-key": fixture.credentials.set("endpoint:endpoint:original", "different-key")
            case "disabled-profile": try fixture.save(enabled: false)
            case "disabled-endpoint": try fixture.save(endpointEnabled: false)
            case "changed-endpoint": try fixture.save(endpoint: "https://different.invalid/v1")
            case "changed-model": try fixture.save(model: "different-model")
            case "changed-protocol": try fixture.save(wireProtocol: "responses")
            case "changed-account":
                fixture.credentials.set("endpoint:endpoint:replacement", "fixture-key")
                try fixture.save(account: "endpoint:endpoint:replacement")
            default: try fixture.save(deleted: true)
            }
            var dispatches = 0
            XCTAssertThrowsError(try MacLocalAgentCredentialGate.launch(selection: selection, isResume: true,
                store: fixture.store, operation: { dispatches += 1 }), change)
            XCTAssertEqual(dispatches, 0, change)
        }
    }

    func testCredentialGateDispatchAnonymousTemporaryNativeAndFallbackRules() throws {
        let fixture = try Fixture()
        defer { fixture.close() }

        // Exact credential allows dispatch; missing profile or deleted credential fails closed
        let selection = try MacLocalAgentCredentialGate.capture(fixture.request(), store: fixture.store)
        var dispatches = 0
        try MacLocalAgentCredentialGate.launch(selection: selection, isResume: true,
            store: fixture.store, operation: { dispatches += 1 })
        XCTAssertEqual(dispatches, 1)
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.capture(fixture.request(profileID: "missing"), store: fixture.store))

        // Native endpoint does not resolve cloud profile credentials; configured fallbacks are required
        let unowned = MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:12345/v1",
            apiKey: "unowned", modelID: "native-model", contextWindowTokens: 4096, profileRevision: 1)
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.capture(fixture.request(profileID: "not-a-cloud-profile"),
            localEndpoint: unowned, store: fixture.store))
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.capture(fixture.request(profileID: "not-a-cloud-profile"),
            store: fixture.store))
        let fallback = MacRemoteModelFallback(profileID: "missing-fallback", endpointName: "Fallback",
            endpoint: "https://fallback.invalid/v1", apiKey: "fixture-fallback", model: "fallback")
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.capture(
            fixture.request(fallbacks: [fallback]), store: fixture.store))

        // Protocol changed during preparation cannot be captured as the original route
        let preProtocolChangeRequest = try fixture.request()
        try fixture.save(wireProtocol: "responses")
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.capture(preProtocolChangeRequest, store: fixture.store))

        // Restore chat_completions, then verify deleted key rejection, explicit anonymous, and temporary non-resumable credentials
        try fixture.save(wireProtocol: "chat_completions")
        fixture.credentials.set("endpoint:endpoint:original", nil)
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.capture(fixture.request(), store: fixture.store))

        try fixture.save(account: "")
        let anonymous = try MacLocalAgentCredentialGate.capture(fixture.request(key: ""), store: fixture.store)
        try MacLocalAgentCredentialGate.launch(selection: anonymous, isResume: true,
            store: fixture.store, operation: { dispatches += 1 })
        XCTAssertEqual(dispatches, 2)
        let temporary = try MacLocalAgentCredentialGate.capture(fixture.request(profileID: nil), store: fixture.store)
        try MacLocalAgentCredentialGate.launch(selection: temporary, isResume: false,
            store: fixture.store, operation: { dispatches += 1 })
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.launch(selection: temporary, isResume: true,
            store: fixture.store, operation: { dispatches += 1 }))
        XCTAssertEqual(dispatches, 3)
    }
}

@MainActor
final class MacLocalAgentProcessSessionTests: XCTestCase {
    private enum InjectedWriteFailure: Error { case unavailable }

    private struct DescriptorIdentity {
        let number: Int32
        let device: dev_t
        let inode: ino_t
        init(_ number: Int32) {
            self.number = number
            var info = stat()
            precondition(fstat(number, &info) == 0)
            device = info.st_dev
            inode = info.st_ino
        }
    }

    private struct Fixture {
        let process: Process
        let input: Pipe
        let output: Pipe
        let error: Pipe
        let session: MacLocalAgentProcessSession
        let descriptors: [DescriptorIdentity]

        init(
            registry: MacOwnedChildProcessRegistry,
            executable: String = "/bin/cat",
            writer: @escaping @Sendable (FileHandle, Data) throws -> Void = {
                try $0.write(contentsOf: $1)
            }
        ) {
            process = Process()
            process.executableURL = URL(fileURLWithPath: executable)
            input = Pipe()
            output = Pipe()
            error = Pipe()
            descriptors = [input, output, error].flatMap {
                [DescriptorIdentity($0.fileHandleForReading.fileDescriptor), DescriptorIdentity($0.fileHandleForWriting.fileDescriptor)]
            }
            session = MacLocalAgentProcessSession(
                process: process, inputPipe: input, outputPipe: output, errorPipe: error,
                registry: registry, writeInitial: writer
            )
        }
    }

    private func ticket(_ generation: UInt64 = 1) -> MacRunSubmissionTicket {
        .init(identity: .init(conversationID: "isolated-process", submissionID: "run-\(generation)",
                              generationID: generation))
    }

    private func assertClosed(_ fixture: Fixture, file: StaticString = #filePath, line: UInt = #line) {
        for descriptor in fixture.descriptors {
            var current = stat()
            errno = 0
            if fstat(descriptor.number, &current) == -1 {
                XCTAssertEqual(errno, EBADF, file: file, line: line)
            } else {
                XCTAssertFalse(current.st_dev == descriptor.device && current.st_ino == descriptor.inode,
                    "owned pipe \(descriptor.number) is still open", file: file, line: line)
            }
        }
    }

    func testInitialWriteFailureAfterLaunchDrainsOwnedChildWithoutInventingPreDispatchRejection() async throws {
        let registry = MacOwnedChildProcessRegistry()
        let submission = ticket()
        var fixture: Fixture? = Fixture(registry: registry, writer: { _, _ in
            throw InjectedWriteFailure.unavailable
        })
        weak var child: Process?
        child = fixture?.process
        try fixture!.session.launch(ticket: submission)
        XCTAssertTrue(fixture!.process.isRunning)
        XCTAssertEqual(submission.phase, .dispatching)
        XCTAssertThrowsError(try fixture!.session.writeInitialRequest(Data("request\n".utf8)))
        XCTAssertEqual(submission.phase, .dispatching, "a started child may have observed dispatch")
        await fixture!.session.shutdownAndDrain()
        XCTAssertFalse(fixture!.process.isRunning)
        XCTAssertTrue(fixture!.session.isReleased)
        assertClosed(fixture!)
        XCTAssertEqual(submission.phase, .dispatching)
        fixture = nil
        for _ in 0..<100 where child != nil {
            try await Task.sleep(for: .milliseconds(10))
        }
        withExtendedLifetime(registry) {
            XCTAssertNil(child, "the isolated ownership registry must release the exited child")
        }
    }

    func testPreDispatchRejectionsAndAbandonedTicketsCloseOwnedPipes() async {
        let registry = MacOwnedChildProcessRegistry()
        let missingFixture = Fixture(registry: registry,
            executable: "/nola-test-missing-executable-\(UUID().uuidString)")
        let missingSubmission = ticket(1)
        XCTAssertThrowsError(try missingFixture.session.launch(ticket: missingSubmission))
        XCTAssertEqual(missingSubmission.phase, .rejectedBeforeDispatch)
        XCTAssertFalse(missingFixture.process.isRunning)
        await missingFixture.session.shutdownAndDrain()
        XCTAssertTrue(missingFixture.session.isReleased)
        assertClosed(missingFixture)

        let abandonedFixture = Fixture(registry: registry)
        let abandonedSubmission = ticket(2)
        XCTAssertTrue(abandonedSubmission.cancelPreparation())
        XCTAssertThrowsError(try abandonedFixture.session.launch(ticket: abandonedSubmission))
        XCTAssertEqual(abandonedSubmission.phase, .abandoned)
        XCTAssertFalse(abandonedFixture.process.isRunning)
        await abandonedFixture.session.shutdownAndDrain()
        XCTAssertTrue(abandonedFixture.session.isReleased)
        assertClosed(abandonedFixture)
    }

    func testSessionReplacementCleanupAndNormalExitIsolateOwnedChildren() async throws {
        let registry = MacOwnedChildProcessRegistry()
        let old = Fixture(registry: registry)
        try old.session.launch(ticket: ticket(1))
        await old.session.shutdownAndDrain()
        XCTAssertTrue(old.session.isReleased)

        let replacement = Fixture(registry: registry)
        let nextTicket = ticket(2)
        try replacement.session.launch(ticket: nextTicket)
        _ = old.session.releaseResources()
        await old.session.shutdownAndDrain()
        XCTAssertTrue(replacement.process.isRunning)
        XCTAssertFalse(replacement.session.isReleased)
        XCTAssertEqual(nextTicket.phase, .dispatching)
        XCTAssertNoThrow(try replacement.session.writeInitialRequest(Data("still owned\n".utf8)))

        let finished = Fixture(registry: registry, executable: "/usr/bin/true")
        try finished.session.launch(ticket: ticket(3))
        let exit = await finished.session.waitForExit()
        XCTAssertEqual(exit.status, 0)
        XCTAssertFalse(exit.signaled)
        _ = finished.session.releaseResources()
        XCTAssertTrue(finished.session.isReleased)
        assertClosed(finished)
        await finished.session.shutdownAndDrain()
        XCTAssertTrue(replacement.process.isRunning)

        await replacement.session.shutdownAndDrain()
        XCTAssertFalse(replacement.process.isRunning)
        assertClosed(replacement)
    }
}

final class MacLocalAgentResumeAuthorizationAndFrameTests: XCTestCase {
    private func rule(_ resource: String, decision: String = "allow") -> MacCanonicalJSONValue {
        .object(["capability_pattern": .string("render_ui"), "resource_pattern": .string(resource),
                 "decision": .string(decision), "source": .string("conversation")])
    }

    private func original() -> [String: Any] {
        ["prompt": "Do not submit this twice", "submission_id": "run",
         "user_message_id": "user", "assistant_message_id": "run:assistant",
         "context_components": [["kind": "platform_data", "content": "old context"]],
         "input_origin": "interactive_user", "attachments": [["path": "/fixture/a.txt", "byte_count": 10]],
         "permission_profile": "safe", "permission_rules": [],
         "model_protocol": "responses", "max_output_tokens": 64000,
         "reasoning_selection": ["kind": "effort", "effort": "high"],
         "reasoning_tier": "concise",
         "external_session_id": "original-executor-session", "api_key": "fixture-not-a-secret"]
    }

    func testFrozenExecutorIdentityWorkspaceScopePermissionModeAndSymlinkValidation() throws {
        let route: [String: Any] = ["kind": "external", "backend": "codex",
            "model": "gpt-6.1-sol", "provider": NSNull()]
        let identity = try XCTUnwrap(MacFrozenExternalExecutorIdentity.resolve(primary: route,
            modelProfileID: nil, fallbackCount: 0))
        XCTAssertEqual(identity.backend, "codex")
        XCTAssertEqual(identity.model, "gpt-6.1-sol")
        XCTAssertNil(identity.provider)
        XCTAssertThrowsError(try MacFrozenExternalExecutorIdentity.resolve(primary: route, modelProfileID: "http-profile", fallbackCount: 0))
        XCTAssertThrowsError(try MacFrozenExternalExecutorIdentity.resolve(primary: route, modelProfileID: nil, fallbackCount: 1))
        var invalid = route
        invalid["backend"] = "nola"
        XCTAssertThrowsError(try MacFrozenExternalExecutorIdentity.resolve(primary: invalid, modelProfileID: nil, fallbackCount: 0))
        invalid = route
        invalid["endpoint"] = "https://unexpected.invalid"
        XCTAssertThrowsError(try MacFrozenExternalExecutorIdentity.resolve(primary: invalid, modelProfileID: nil, fallbackCount: 0))
        XCTAssertNil(try MacFrozenExternalExecutorIdentity.resolve(primary: ["kind": "http"], modelProfileID: "profile", fallbackCount: 0))

        let project = URL(fileURLWithPath: "/fixture/project")
        let originalDir = URL(fileURLWithPath: "/fixture/original")
        let added = URL(fileURLWithPath: "/fixture/added")
        let frozen = MacLocalAgentFrozenAuthorization(projectURL: project,
            authorizedWorkspaceURLs: [originalDir], permissionMode: .cautious)
        XCTAssertNoThrow(try frozen.validate(.init(projectURL: project, authorizedWorkspaceURLs: [originalDir, added], permissionMode: .cautious)))
        XCTAssertThrowsError(try frozen.validate(.init(projectURL: project, authorizedWorkspaceURLs: [added], permissionMode: .cautious)))
        XCTAssertThrowsError(try frozen.validate(.init(projectURL: added, authorizedWorkspaceURLs: [project, originalDir], permissionMode: .cautious)))
        XCTAssertNoThrow(try frozen.validate(.init(projectURL: project, authorizedWorkspaceURLs: [originalDir], permissionMode: .cautious)))
        XCTAssertNoThrow(try frozen.validate(.init(projectURL: project, authorizedWorkspaceURLs: [originalDir], permissionMode: .allowAll)))

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
        XCTAssertEqual(cautious.permission_mode, "cautious")
        let full = MacLocalAgentFrozenAuthorization(projectURL: project, authorizedWorkspaceURLs: [], permissionMode: .allowAll)
        XCTAssertThrowsError(try full.validate(.init(projectURL: project, authorizedWorkspaceURLs: [], permissionMode: .cautious)))
        XCTAssertThrowsError(try cautious.validate(.init(projectURL: project.appendingPathComponent("other"), authorizedWorkspaceURLs: [project], permissionMode: .allowAll)))

        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let symlinkOrig = root.appendingPathComponent("original")
        let symlinkRepl = root.appendingPathComponent("replacement")
        let link = root.appendingPathComponent("granted-link")
        for url in [symlinkOrig, symlinkRepl] {
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        }
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: symlinkOrig)
        let symlinkFrozen = MacLocalAgentFrozenAuthorization(projectURL: root, authorizedWorkspaceURLs: [link], permissionMode: .cautious)
        XCTAssertNoThrow(try symlinkFrozen.validate(.init(projectURL: root, authorizedWorkspaceURLs: [link], permissionMode: .cautious)))
        try FileManager.default.removeItem(at: link)
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: symlinkRepl)
        XCTAssertThrowsError(try symlinkFrozen.validate(.init(projectURL: root, authorizedWorkspaceURLs: [symlinkOrig, symlinkRepl, link], permissionMode: .cautious)))
    }

    func testColdResumePermissionRulesAndFrameTransformationContract() throws {
        let initial = rule("create")
        let later = rule("update")
        try MacFrozenLaunchDescriptor.validatePermissionRules(frozen: [initial], current: [later, initial])
        try MacFrozenLaunchDescriptor.validatePermissionRules(frozen: [initial, later], current: [later, initial])
        try MacFrozenLaunchDescriptor.validatePermissionRules(frozen: [initial, initial], current: [initial])
        try MacFrozenLaunchDescriptor.validatePermissionRules(frozen: [initial], current: [initial, initial])

        let changedSource: MacCanonicalJSONValue = .object(["capability_pattern": .string("render_ui"),
            "resource_pattern": .string("create"), "decision": .string("allow"), "source": .string("global")])
        for current in [[], [changedSource], [rule("update")], [initial, rule("*", decision: "deny")],
                        [initial, rule("*", decision: "ask")], [initial, .null]] as [[MacCanonicalJSONValue]] {
            XCTAssertThrowsError(try MacFrozenLaunchDescriptor.validatePermissionRules(frozen: [initial], current: current))
        }

        let source = try JSONSerialization.data(withJSONObject: original())
        let frame = try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: source,
            conversationID: "conversation", runID: "run", assistantMessageID: "run:assistant")
        XCTAssertEqual(frame.last, 0x0a)
        let envelope = try XCTUnwrap(JSONSerialization.jsonObject(with: frame) as? [String: Any])
        XCTAssertEqual(Set(envelope.keys), ["operation", "session_id", "expected_run_id", "assistant_message_id", "configuration", "expert_template_authority"])
        XCTAssertEqual(envelope["operation"] as? String, "resume")
        XCTAssertEqual(envelope["session_id"] as? String, "conversation")
        XCTAssertEqual(envelope["expected_run_id"] as? String, "run")
        let configuration = try XCTUnwrap(envelope["configuration"] as? [String: Any])
        for key in ["prompt", "submission_id", "user_message_id", "assistant_message_id", "context_components", "selected_experts", "expected_source_store_epoch", "expert_template_authority", "input_origin", "attachments"] {
            XCTAssertNil(configuration[key], key)
        }
        XCTAssertEqual(configuration["max_output_tokens"] as? Int, 64000)
        XCTAssertEqual(configuration["reasoning_tier"] as? String, "concise")
        XCTAssertEqual(configuration["external_session_id"] as? String, "original-executor-session")
        XCTAssertEqual(configuration["permission_profile"] as? String, "safe")
        XCTAssertEqual(configuration["host_attachments"] as? [[String: AnyHashable]], original()["attachments"] as? [[String: AnyHashable]])
        XCTAssertEqual((try JSONSerialization.jsonObject(with: source) as? [String: Any])?["prompt"] as? String, "Do not submit this twice")

        var expected = original()
        for key in ["prompt", "submission_id", "user_message_id", "assistant_message_id", "context_components", "input_origin"] {
            expected.removeValue(forKey: key)
        }
        expected["host_attachments"] = expected.removeValue(forKey: "attachments")
        XCTAssertEqual(try JSONSerialization.data(withJSONObject: configuration, options: .sortedKeys),
                       try JSONSerialization.data(withJSONObject: expected, options: .sortedKeys))

        XCTAssertThrowsError(try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: source,
            conversationID: "conversation", runID: "replacement", assistantMessageID: "run:assistant"))
        XCTAssertThrowsError(try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: source,
            conversationID: "conversation", runID: "run", assistantMessageID: "other:assistant"))

        var forged = original()
        forged["expert_template_authority"] = ["actor": ["kind": "forged_web_actor"], "request_host_id": "foreign"]
        forged["selected_experts"] = [["agent_id": "expert", "agent_revision": 1]]
        forged["expected_source_store_epoch"] = "old-host-store"
        let forgedFrame = try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: JSONSerialization.data(withJSONObject: forged),
            conversationID: "conversation", runID: "run", assistantMessageID: "run:assistant")
        let forgedValue = try XCTUnwrap(JSONSerialization.jsonObject(with: forgedFrame) as? [String: Any])
        let authority = try XCTUnwrap(forgedValue["expert_template_authority"] as? [String: Any])
        XCTAssertTrue(authority["request_host_id"] is NSNull)
        let actor = try XCTUnwrap(authority["actor"] as? [String: Any])
        XCTAssertEqual(actor["kind"] as? String, "local_user")
        XCTAssertFalse((actor["id"] as? String ?? "").isEmpty)
        let forgedConfig = try XCTUnwrap(forgedValue["configuration"] as? [String: Any])
        XCTAssertNil(forgedConfig["expert_template_authority"])
        XCTAssertNil(forgedConfig["selected_experts"])
        XCTAssertNil(forgedConfig["expected_source_store_epoch"])

        var automationInput = original()
        automationInput["automation_execution_lease"] = ["lease_token": "one-admission-only"]
        automationInput["automation_execution_spec"] = ["prompt": "original"]
        automationInput["automation_artifacts"] = [["path": "/mutable/source", "sha256": "original"]]
        automationInput["mac_launch"] = ["model_profile_id": "frozen-profile", "model_profile_revision": NSNull(), "input_origin": "scheduled_action"]
        let automationFrame = try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: JSONSerialization.data(withJSONObject: automationInput),
            conversationID: "conversation", runID: "run", assistantMessageID: "run:assistant")
        let automationDecoded = try XCTUnwrap(JSONSerialization.jsonObject(with: automationFrame) as? [String: Any])
        let automationConfig = try XCTUnwrap(automationDecoded["configuration"] as? [String: Any])
        for key in ["automation_execution_lease", "automation_execution_spec", "automation_artifacts", "prompt"] {
            XCTAssertNil(automationConfig[key])
        }
        XCTAssertEqual((automationConfig["mac_launch"] as? [String: Any])?["model_profile_id"] as? String, "frozen-profile")
    }
}

struct MacLocalHostPromptBoundaryTests {
    @Test func localAndRemoteRunsPreserveTypedContextWithoutSystemPromotion() throws {
        let localComponents = [
            MacCoreContextComponent(kind: "memory", id: "memory-1", content: "Untrusted recalled context"),
            MacCoreContextComponent(kind: "prompt_template", id: "template-1", content: "User-selected instruction"),
        ]
        let request = try MacLocalAgentRunRequest(
            conversationID: "prompt-boundary",
            prompt: "Hello",
            contextComponents: localComponents,
            projectURL: FileManager.default.temporaryDirectory,
            attachmentURLs: []
        )
        #expect(request.contextComponents == localComponents)
        #expect(request.contextComponents.map(\.kind) == ["memory", "prompt_template"])
        #expect(!request.contextComponents.contains { $0.kind == "system" })

        let remoteComponents = [
            MacCoreContextComponent(kind: "memory", id: "memory-remote", content: "Recalled context"),
            MacCoreContextComponent(kind: "attachment", id: "attachment-remote", content: "Document contents"),
        ]
        let payload = try MacRemoteContextPayload.encode(remoteComponents)
        #expect(payload.count == 2)
        #expect(payload.compactMap { $0["kind"] as? String } == ["memory", "attachment"])
        #expect(!payload.contains { ($0["kind"] as? String) == "system" })
    }
}

@MainActor
struct MacPlatformContextAuthorizationTests {
    @Test func platformContextPreservesAuthorizationObservationAndUnspecifiedDefault() {
        let observed = Date(timeIntervalSince1970: 1_788_640_800)
        let availableItem = MacPlatformDataContext(sourceID: "contacts", title: "Contacts", content: "Fixture", observedAt: observed, authorization: .available)
        let component = availableItem.coreComponent
        #expect(component.source?.authorization == "available")
        #expect(component.source?.observedAt == ISO8601DateFormatter().string(from: observed))
        #expect(component.content == .string("Fixture"))

        let unspecifiedItem = MacPlatformDataContext(sourceID: "contacts", title: "Contacts", content: "Fixture", observedAt: nil)
        #expect(unspecifiedItem.coreComponent.source?.authorization == "notDetermined")
    }
}

@MainActor
final class MacLocalRunModelRouteTests: XCTestCase {
    func testCapturedRouteKeepsIdentityAfterEditsAndEnforcesCoolingRules() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-run-model-route-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = MacAppleChatConfigurationStore(
            applicationPort: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        )
        var endpoint = MacRemoteEndpointConfiguration(
            id: "endpoint", name: "Before", baseURL: "https://before.invalid/v1",
            models: [MacRemoteModelConfiguration(
                id: "profile", modelName: "original-model",
                executionCapabilities: MacModelExecutionCapabilities(
                    toolCalls: true, imageInput: false, reasoning: false, samplingParameters: true
                )
            )]
        )
        try store.replaceEndpoints([endpoint])
        let captured = MacLocalRunModelRoute(
            configuredProvider: try XCTUnwrap(store.load(model: "profile")),
            routePlan: try XCTUnwrap(store.routePlan(preferredModel: "profile"))
        )

        endpoint.baseURL = "https://after.invalid/v1"
        endpoint.wireProtocol = .responses
        endpoint.models[0].modelName = "replacement-model"
        endpoint.models[0].executionCapabilities.samplingParameters = false
        endpoint.models[0].confirmExecutionCapabilities()
        try store.replaceEndpoints([endpoint])
        store.markQuotaExhausted(profileID: "profile", until: .now.addingTimeInterval(3_600))

        XCTAssertEqual(store.load(model: "profile")?.model, "replacement-model")
        XCTAssertNil(store.routePlan(preferredModel: "profile"))
        let frozen = try XCTUnwrap(captured.validatedProvider(requiresSelectedProfile: true))
        XCTAssertEqual(frozen.profileID, "profile")
        XCTAssertEqual(frozen.model, "original-model")
        XCTAssertEqual(frozen.baseURL.absoluteString, "https://before.invalid/v1")
        XCTAssertEqual(frozen.wireProtocol, .chatCompletions)
        XCTAssertTrue(frozen.executionCapabilities.samplingParameters)

        let primary = MacAppleChatConfiguration(
            profileID: "selected", baseURL: URL(string: "https://selected.invalid")!,
            apiKey: "", model: "selected"
        )
        let fallback = MacAppleChatConfiguration(
            profileID: "fallback", baseURL: URL(string: "https://fallback.invalid")!,
            apiKey: "", model: "fallback"
        )
        let cooled = MacLocalRunModelRoute(configuredProvider: primary, routePlan: nil)
        XCTAssertThrowsError(try cooled.validatedProvider(requiresSelectedProfile: false))
        let substituted = MacLocalRunModelRoute(
            configuredProvider: primary,
            routePlan: MacRemoteModelRoutePlan(primary: fallback, fallbacks: [])
        )
        XCTAssertThrowsError(try substituted.validatedProvider(requiresSelectedProfile: true))
        XCTAssertEqual(try substituted.validatedProvider(requiresSelectedProfile: false)?.profileID, "fallback")
        let local = MacLocalRunModelRoute(configuredProvider: nil, routePlan: nil)
        XCTAssertNil(try local.validatedProvider(requiresSelectedProfile: false))
    }
}

private final class UnknownRecoveryServiceFixture: @unchecked Sendable {
    private let lock = NSLock()
    let resolutionGate: DispatchSemaphore?
    private var resolutionStarted = false
    init(resolutionGate: DispatchSemaphore? = nil) { self.resolutionGate = resolutionGate }
    var hasStartedResolution: Bool {
        lock.lock(); defer { lock.unlock() }; return resolutionStarted
    }
    private var authorized = false
    private var submitted: [MacUnknownEffectRecoveryDecision] = []
    var decisions: [MacUnknownEffectRecoveryDecision] {
        lock.lock(); defer { lock.unlock() }; return submitted
    }
    func execute(_ data: Data, _ requestID: String) throws -> Data {
        let command = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        if command["type"] as? String == "resolve_unknown_effect_recovery", let resolutionGate {
            lock.lock(); resolutionStarted = true; lock.unlock()
            guard resolutionGate.wait(timeout: .now() + 5) == .success else {
                throw MacApplicationServiceError.connectionFailed("fixture resolution gate timed out")
            }
        }
        lock.lock(); defer { lock.unlock() }
        switch command["type"] as? String {
        case "inspect_unknown_effect_recovery":
            if authorized { return Data("null".utf8) }
            return try JSONEncoder().encode(MacUnknownEffectRecoveryInspection(
                conversationID: try XCTUnwrap(command["conversation_id"] as? String),
                runID: try XCTUnwrap(command["expected_run_id"] as? String),
                assistantMessageID: try XCTUnwrap(command["assistant_message_id"] as? String),
                invocationConfigDigest: "fixture-digest", targets: [.init(effectID: "effect-1",
                    requestID: "request-1", attemptID: "attempt-1", workerGeneration: "worker-1",
                    leaseOwner: "owner", leaseToken: "private-token", expectedAttemptRevision: 1)],
                retryAllowed: true, independentPendingEffects: 0))
        case "resolve_unknown_effect_recovery":
            let input = try JSONSerialization.data(withJSONObject: XCTUnwrap(command["input"]))
            let decision = try JSONDecoder().decode(MacUnknownEffectRecoveryDecision.self, from: input)
            submitted.append(decision)
            authorized = true
            return try JSONSerialization.data(withJSONObject: ["outcome": "retry_authorized",
                "resolution_id": decision.resolutionID, "run_id": decision.runID])
        default: throw MacApplicationServiceError.invalidResponse
        }
    }
}

/// Only the transport is scripted. Run creation, effect ownership, model
/// completion and terminal queries execute the real Application transactions.
private actor TerminalContractLocalRuntime: MacLocalAgentRuntime {
    enum StreamEnd: Sendable { case eof, cancelled, failed }
    enum FixtureError: Error { case transport, preparation, invalidContract }
    struct RequestSnapshot: Equatable, Sendable {
        let conversationID: String
        let runID: String
        let assistantID: String
        let prompt: String
        let reasoningTier: String?
        let modelID: String?
        let contextWindowTokens: Int?
    }
    private struct Run {
        let request: MacLocalAgentRunRequest
        let client: MacApplicationServiceClient
        let digest: String
        var modelRequest: [String: Any]
        var continuation: AsyncThrowingStream<MacLocalAgentEvent, Error>.Continuation
    }
    private var runs: [String: Run] = [:]
    private var order: [String] = []
    private var preparationFailure = false
    private var pausesBeforeAdmission = false
    private var pendingAdmission: (MacRunSubmissionTicket, AsyncThrowingStream<MacLocalAgentEvent, Error>.Continuation)?
    private var failure: String?
    private var cancellationCount = 0
    private var cancellationDeliveryFailure = false
    private var cancellationRequests: [MacRunCancellationRequest] = []
    private var ownedTransports: Set<String> = []
    private var interruptedRuns: [String] = []
    private var resumeFailure = false
    private var resumedRequests: [RequestSnapshot] = []
    private var beforeRevalidate: (@MainActor @Sendable () -> Void)?
    private var pauseBeforeReady = false
    private var readyGateEntered = false
    private var readyGate: CheckedContinuation<Void, Never>?
    private var completedResumeAttempts = 0

    func setBeforeRevalidate(_ hook: @escaping @MainActor @Sendable () -> Void) { beforeRevalidate = hook }
    func pauseNextReady() { pauseBeforeReady = true }
    func isWaitingForReady() -> Bool { readyGateEntered }
    func releaseReady() { readyGate?.resume(); readyGate = nil }
    func finishedResumeAttempts() -> Int { completedResumeAttempts }

    func failPreparation() { preparationFailure = true }
    func pauseBeforeAdmission() { pausesBeforeAdmission = true }
    func hasPendingAdmission() -> Bool { pendingAdmission != nil }
    func finishPendingAdmission(rejected: Bool) {
        guard let (ticket, continuation) = pendingAdmission else { return }
        pendingAdmission = nil
        if rejected {
            _ = ticket.rejectBeforeAdmission()
            continuation.finish(throwing: MacLocalAgentRuntimeError.submissionRejected(
                code: "request_rejected_before_admission", message: "Host invocation unavailable: incompatible checkpoint"))
        } else {
            continuation.finish(throwing: FixtureError.transport)
        }
    }
    func runIDs() -> [String] { order }
    func capturedRequest(_ id: String) -> MacLocalAgentRunRequest? { runs[id]?.request }
    func setupFailure() -> String? { failure }
    func cancelCount() -> Int { cancellationCount }
    func failNextCancellationDelivery() { cancellationDeliveryFailure = true }
    func deliveredCancellationRequests() -> [MacRunCancellationRequest] { cancellationRequests }
    func interruptedRunIDs() -> [String] { interruptedRuns }
    func failNextResume() { resumeFailure = true }
    func resumeSnapshots() -> [RequestSnapshot] { resumedRequests }
    func requestSnapshot(_ id: String) -> RequestSnapshot? {
        guard let request = runs[id]?.request else { return nil }
        return RequestSnapshot(conversationID: request.conversationID, runID: request.submissionID,
            assistantID: request.assistantMessageID, prompt: request.prompt, reasoningTier: request.reasoningTier,
            modelID: request.modelID, contextWindowTokens: request.contextWindowTokens)
    }
    func canResume(conversationID: String, expectedRunID: String, assistantMessageID: String) -> Bool {
        guard let run = runs[expectedRunID] else { return false }
        return run.request.conversationID == conversationID && run.request.assistantMessageID == assistantMessageID
    }
    func resume(conversationID: String, expectedRunID: String, assistantMessageID: String,
                revalidate: @escaping @MainActor @Sendable () async throws -> MacLocalAgentResumeAuthorization,
                transportReady: (@MainActor @Sendable () async -> Void)?) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { continuation in
            Task {
                defer { completedResumeAttempts += 1 }
                do {
                    guard canResume(conversationID: conversationID, expectedRunID: expectedRunID,
                                    assistantMessageID: assistantMessageID) else { throw FixtureError.invalidContract }
                    await beforeRevalidate?()
                    beforeRevalidate = nil
                    _ = try await revalidate()
                    guard var run = runs[expectedRunID], let snapshot = requestSnapshot(expectedRunID) else {
                        throw FixtureError.invalidContract
                    }
                    resumedRequests.append(snapshot)
                    if resumeFailure { resumeFailure = false; throw FixtureError.transport }
                    // Reattach the fake transport to the SAME real, already
                    // committed Core Run. Never execute begin_assistant_turn
                    // or submit its prompt a second time on this path.
                    run.continuation = continuation
                    runs[expectedRunID] = run
                    ownedTransports.insert(expectedRunID)
                    if pauseBeforeReady {
                        pauseBeforeReady = false
                        await withCheckedContinuation { readyGateEntered = true; readyGate = $0 }
                    }
                    await transportReady?()
                } catch { continuation.finish(throwing: error) }
            }
        }
    }

    func run(_ request: MacLocalAgentRunRequest) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { continuation in
            do {
                if preparationFailure {
                    request.submissionTicket?.rejectBeforeTransport()
                    throw FixtureError.preparation
                }
                try request.submissionTicket?.claimDispatch()
                if pausesBeforeAdmission, let ticket = request.submissionTicket {
                    pendingAdmission = (ticket, continuation)
                    return
                }
                guard let database = request.databaseURLOverride else { throw FixtureError.invalidContract }
                let client = try MacApplicationServiceClient(databaseURL: database)
                let now = Int64(Date().timeIntervalSince1970 * 1_000)
                let capability: [String: Any] = [
                    "id": "model.stream", "title": "Fixture model", "route": "model", "availability": "available",
                    "exposure": "internal", "input_schema": ["type": "object"],
                    "output_policy": ["preview_char_limit": 4000, "persist_full_output": true, "allow_artifacts": true],
                    "supports_streaming": true, "parallelism": "serial", "cancellation": "cooperative",
                    "replay_safety": "recoverable"
                ]
                let accepted = try Self.command(client, ["type": "begin_assistant_turn", "input": [
                    "conversation_id": request.conversationID, "conversation_title": "Local terminal fixture",
                    "expected_configuration_revision": request.expectedConfigurationRevision as Any? ?? NSNull(),
                    "expected_parent_message_id": request.expectedParentMessageID as Any? ?? NSNull(),
                    "user_message_id": request.userMessageID, "user_part_id": "\(request.userMessageID):text",
                    "user_text": request.prompt, "assistant_message_id": request.assistantMessageID,
                    "core_config": ["session_id": request.conversationID, "model": "fixture-model",
                        "turn_identity": ["model_capability_id": "mac-journal-fixture",
                            "model_capability_revision": 1,
                            "model_capability": MacAssistantJournalTestFixture.modelCapabilityProfile(
                                toolCapabilityID: nil, contextWindow: 128_000, maxOutput: 4096)],
                        "host_catalog": ["host_id": "fixture-local-host", "revision": 1, "availability_epoch": 1,
                            "capabilities": ["model.stream": capability]]],
                    "submission": ["type": "user_input", "items": [["type": "message", "id": request.userMessageID,
                        "role": "user", "content": request.prompt]], "submission_id": request.submissionID,
                        "submitted_at_epoch_ms": now, "delivery": "current_turn"], "created_at_ms": now
                ]])
                guard let digest = accepted["invocation_config_digest"] as? String else { throw FixtureError.invalidContract }
                let claimed = try Self.claim(client, runID: request.submissionID)
                guard var model = claimed.first(where: { ($0["payload"] as? [String: Any])?["type"] as? String == "model_stream" }) else {
                    throw FixtureError.invalidContract
                }
                _ = try Self.accept(client, request: request, digest: digest, host: model, sequence: 1,
                    payload: ["type": "execution_started"])
                _ = try Self.accept(client, request: request, digest: digest, host: model, sequence: 2,
                    payload: ["type": "model_attempt_started", "provider": "fixture", "model": "fixture-model", "reason": "initial"])
                let loaded = try Self.command(client, ["type": "load_committed_host_requests", "run_id": request.submissionID])
                guard let refreshed = (loaded["requests"] as? [[String: Any]])?.first(where: {
                    $0["effect_id"] as? String == model["effect_id"] as? String
                }) else { throw FixtureError.invalidContract }
                let delivery = model["delivery"]
                model = refreshed
                model["delivery"] = delivery
                runs[request.submissionID] = Run(request: request, client: client, digest: digest,
                    modelRequest: model, continuation: continuation)
                order.append(request.submissionID)
                ownedTransports.insert(request.submissionID)
                request.submissionTicket?.markAccepted()
                Task { await request.submissionAccepted?() }
            } catch {
                failure = String(reflecting: error)
                continuation.finish(throwing: error)
            }
        }
    }

    func end(_ id: String, as mode: StreamEnd) throws {
        guard let run = runs[id] else { throw FixtureError.invalidContract }
        ownedTransports.remove(id)
        switch mode {
        case .eof: run.continuation.finish()
        case .cancelled: run.continuation.finish(throwing: CancellationError())
        case .failed: run.continuation.finish(throwing: FixtureError.transport)
        }
    }

    func commitCompletion(_ id: String, answer: String = "Committed local answer") throws {
        guard let run = runs[id], let payload = run.modelRequest["payload"] as? [String: Any],
              let partID = payload["text_part_id"] as? String else { throw FixtureError.invalidContract }
        _ = try Self.accept(run.client, request: run.request, digest: run.digest, host: run.modelRequest, sequence: 3,
            payload: ["type": "model_completed", "items": [["type": "message", "id": partID,
                "role": "assistant", "content": answer]], "tool_calls": [], "usage": [:]])
        for hook in try Self.claim(run.client, runID: id) {
            guard (hook["payload"] as? [String: Any])?["type"] as? String == "run_lifecycle_hook" else {
                throw FixtureError.invalidContract
            }
            _ = try Self.accept(run.client, request: run.request, digest: run.digest, host: hook, sequence: 1,
                payload: ["type": "execution_started"])
            _ = try Self.accept(run.client, request: run.request, digest: run.digest, host: hook, sequence: 2,
                payload: ["type": "completed", "output": [:], "artifacts": []])
        }
    }

    func emitOldCompletion(into id: String, oldRunID: String) throws {
        guard let run = runs[id] else { throw FixtureError.invalidContract }
        run.continuation.yield(MacLocalAgentEvent(kind: .finished, text: "", title: nil, detail: nil,
            isError: false).bindingCanonicalRun(oldRunID))
    }

    func cancellationFacts(_ id: String) throws -> (status: String, revision: Int, pendingCancels: Int) {
        guard let run = runs[id] else { throw FixtureError.invalidContract }
        let record = try Self.command(run.client, ["type": "load_checkpoint", "conversation_id": run.request.conversationID])
        guard let json = record["checkpoint_json"] as? String,
              let data = json.data(using: .utf8),
              let checkpoint = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let state = checkpoint["state"] as? [String: Any],
              let execution = state["execution"] as? [String: Any],
              let turn = execution["active_turn"] as? [String: Any],
              let status = turn["status"] as? String,
              let revision = record["revision"] as? Int else { throw FixtureError.invalidContract }
        let requests = try Self.command(run.client, ["type": "load_committed_host_requests", "run_id": id])
        let pending = (requests["requests"] as? [[String: Any]] ?? []).filter {
            ($0["payload"] as? [String: Any])?["type"] as? String == "cancel"
        }.count
        return (status, revision, pending)
    }

    /// Explicit isolated Host evidence for fixture cleanup; this is not a
    /// claim that production cancel-only transport has been implemented.
    func commitCancellation(_ id: String) throws {
        guard let run = runs[id] else { throw FixtureError.invalidContract }
        for _ in 0..<4 {
            let requests = try Self.claim(run.client, runID: id)
            if requests.isEmpty { return }
            for request in requests {
                guard let payload = request["payload"] as? [String: Any],
                      let kind = payload["type"] as? String else { throw FixtureError.invalidContract }
                _ = try Self.accept(run.client, request: run.request, digest: run.digest, host: request,
                    sequence: 1, payload: ["type": "execution_started"])
                let result: [String: Any]
                switch kind {
                case "cancel":
                    guard let target = payload["target_effect_id"] as? String else { throw FixtureError.invalidContract }
                    result = ["type": "cancellation_confirmed", "target_effect_id": target]
                case "run_lifecycle_hook": result = ["type": "completed", "output": [:], "artifacts": []]
                default: throw FixtureError.invalidContract
                }
                _ = try Self.accept(run.client, request: run.request, digest: run.digest, host: request,
                    sequence: 2, payload: result)
            }
        }
        throw FixtureError.invalidContract
    }

    private static func command(_ client: MacApplicationServiceClient, _ object: [String: Any]) throws -> [String: Any] {
        let data = try client.execute(commandData: JSONSerialization.data(withJSONObject: object))
        guard let value = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw FixtureError.invalidContract }
        return value
    }
    private static func claim(_ client: MacApplicationServiceClient, runID: String) throws -> [[String: Any]] {
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let value = try command(client, ["type": "claim_committed_host_requests", "input": ["run_id": runID,
            "worker_generation": "test-worker", "lease_owner": "test-owner", "claimed_at_ms": now,
            "lease_expires_at_ms": now + 60_000, "limit": 32]])
        guard let requests = value["requests"] as? [[String: Any]] else { throw FixtureError.invalidContract }
        return requests
    }
    private static func accept(_ client: MacApplicationServiceClient, request: MacLocalAgentRunRequest,
                               digest: String, host: [String: Any], sequence: Int,
                               payload: [String: Any]) throws -> [String: Any] {
        let keys = ["request_id", "attempt_id", "delegated_invocation_id", "effect_id", "host_id", "lease_id",
                    "session_id", "turn_id", "catalog_revision", "availability_epoch", "delivery"]
        var response = host.filter { keys.contains($0.key) }
        response["sequence"] = sequence
        response["payload"] = payload
        return try command(client, ["type": "reduce_and_accept_host_response", "input": [
            "conversation_id": request.conversationID, "run_id": request.submissionID,
            "invocation_config_digest": digest, "response": response,
            "received_at_ms": Int64(Date().timeIntervalSince1970 * 1_000)]])
    }

    func cancel(conversationID: String) { cancellationCount += 1 }
    func interruptOwnedTransport(conversationID: String, expectedRunID: String, assistantMessageID: String) -> Bool {
        guard ownedTransports.contains(expectedRunID), let run = runs[expectedRunID],
              run.request.conversationID == conversationID,
              run.request.assistantMessageID == assistantMessageID else { return false }
        ownedTransports.remove(expectedRunID)
        interruptedRuns.append(expectedRunID)
        run.continuation.finish()
        return true
    }
    func deliverCancellation(_ request: MacRunCancellationRequest, databaseURL: URL) async throws {
        guard let run = runs[request.expectedRunID],
              run.request.conversationID == request.conversationID,
              run.request.assistantMessageID == request.assistantMessageID,
              run.request.databaseURLOverride == databaseURL,
              try cancellationFacts(request.expectedRunID).status == "cancelling" else {
            throw FixtureError.invalidContract
        }
        cancellationRequests.append(request)
        if cancellationDeliveryFailure {
            cancellationDeliveryFailure = false
            throw FixtureError.transport
        }
        cancellationCount += 1
        // Delivery acceptance is deliberately not terminal evidence. Tests
        // separately commit the real Core cancellation response below.
    }
    func resolvePermission(conversationID: String, request: MacRemotePermissionRequest, allow: Bool,
                           scope: MacPermissionGrantScope) throws {}
    func listSessions() async throws -> [MacLocalAgentSessionSummary] { [] }
    func deleteSession(id: String) async throws {}
}

/// Blocks only the bookmark store's detached resolver, never MainActor.
nonisolated private final class LocalTerminalResolutionGate: @unchecked Sendable {
    private let lock = NSLock()
    private let signal = DispatchSemaphore(value: 0)
    private var armed = false
    private var entered = false
    func arm() { lock.lock(); armed = true; entered = false; lock.unlock() }
    var isEntered: Bool { lock.lock(); defer { lock.unlock() }; return entered }
    func resolveBoundary() {
        lock.lock()
        let wait = armed
        if wait { armed = false; entered = true }
        lock.unlock()
        if wait { signal.wait() }
    }
    func release() { signal.signal() }
}

@MainActor
private final class LocalTerminalBookmarkIO: MacWorkspaceBookmarkIO {
    let root: URL
    nonisolated let gate: LocalTerminalResolutionGate
    init(_ root: URL, gate: LocalTerminalResolutionGate) { self.root = root; self.gate = gate }
    func chooseFolder(suggested: URL?) -> URL? { root }
    func bookmark(_ url: URL) throws -> Data { Data(url.absoluteString.utf8) }
    nonisolated func resolve(_ data: Data) throws -> (url: URL, stale: Bool) {
        gate.resolveBoundary()
        guard let text = String(data: data, encoding: .utf8), let url = URL(string: text) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        return (url, false)
    }
    func start(_ url: URL) -> Bool { true }
    func stop(_ url: URL) {}
}

@MainActor
final class MacLocalRunTerminalContractTests: XCTestCase {
    @MainActor
    private final class Fixture {
        let root: URL
        let suite: String
        let defaults: UserDefaults
        let bookmarks: MacSecurityScopedBookmarkStore
        let port: MacApplicationPort
        let repository: MacCanonicalConversationRepository
        let activity: MacActivityStore
        let runtime = TerminalContractLocalRuntime()
        let resolutionGate = LocalTerminalResolutionGate()
        let model: MacWorkspaceModel
        let conversation: String
        init(unknownTransport: MacUnknownEffectRecoveryControl.Transport? = nil) async throws {
            root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            suite = "Nola.LocalTerminal.\(UUID().uuidString)"
            defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
            bookmarks = MacSecurityScopedBookmarkStore(defaults: defaults, io: LocalTerminalBookmarkIO(root, gate: resolutionGate))
            port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
            repository = MacCanonicalConversationRepository(store: port)
            activity = MacActivityStore(baseDirectory: root.appendingPathComponent("activity"))
            let recoveryURL = root.appendingPathComponent("recovery.db")
            model = MacWorkspaceModel(canonicalRepository: repository, localAgentRuntime: runtime,
                remoteHost: PersistenceTestRemoteHost(), modelApplication: port, bookmarkStore: bookmarks,
                unknownEffectRecoveryControl: unknownTransport.map {
                    MacUnknownEffectRecoveryControl(databaseURL: recoveryURL,
                        clientID: "fixture", transport: $0)
                },
                activityStore: activity, completionNotifier: { _, _, _ in },
                contextPreparation: { $0.contextComponents }, readinessIssue: { _ in nil })
            conversation = try XCTUnwrap(model.selectedConversationID)
            let selectedWorkspace = await bookmarks.chooseWorkspace(conversationID: conversation)
            model.workspaceURL = try XCTUnwrap(selectedWorkspace)
            model.executionHost = .local
            model.selectExecutionBackend(.fallback(id: "codex"), available: true)
            model.presentationDidAttach()
        }
        func cleanUp() {
            model.presentationDidDetach()
            bookmarks.close()
            defaults.removePersistentDomain(forName: suite)
            try? FileManager.default.removeItem(at: root)
        }
        func executeSQLite(_ sql: String) throws {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
            process.arguments = [root.appendingPathComponent("store.db").path, sql]
            let errors = Pipe()
            process.standardError = errors
            try process.run()
            let detail = errors.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            XCTAssertEqual(process.terminationStatus, 0, String(decoding: detail, as: UTF8.self))
        }
    }

    private func waitUntil(_ predicate: () async throws -> Bool,
                           file: StaticString = #filePath, line: UInt = #line) async throws {
        let deadline = Date().addingTimeInterval(3)
        while try await !predicate(), Date() < deadline { try await Task.sleep(for: .milliseconds(10)) }
        let satisfied = try await predicate()
        XCTAssertTrue(satisfied, "Expected real local lifecycle boundary", file: file, line: line)
    }

    private func start(_ fixture: Fixture, prompt: String = "Explain the result") async throws -> String {
        let prior = await fixture.runtime.runIDs().count
        fixture.model.draft = prompt
        try await fixture.model.waitForModelConfigurationCommit()
        fixture.model.send()
        try await waitUntil {
            let count = await fixture.runtime.runIDs().count
            let failure = await fixture.runtime.setupFailure()
            return count > prior || failure != nil
        }
        let failure = await fixture.runtime.setupFailure()
        XCTAssertNil(failure)
        let ids = await fixture.runtime.runIDs()
        return try XCTUnwrap(ids.dropFirst(prior).first,
            "This submission must admit its own Run; a predecessor cannot stand in for a rejected Begin")
    }

    func testFirstDraftSubmissionCreatesOwnerBeforeFreezingItsBeginWitness() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        // Keep this a genuinely unpersisted draft. Precreating it in Fixture
        // would hide a history/frontier read performed before Ensure.
        XCTAssertNil(try fixture.port.loadConversation(fixture.conversation))

        let run = try await start(fixture)
        let captured = await fixture.runtime.capturedRequest(run)
        let request = try XCTUnwrap(captured)
        let metadata = try XCTUnwrap(fixture.port.loadConversationMetadata(fixture.conversation))
        XCTAssertEqual(request.conversationID, fixture.conversation)
        XCTAssertNotNil(request.expectedConfigurationRevision)
        XCTAssertEqual(request.expectedConfigurationRevision, metadata.configurationRevision,
            "Begin must carry the new owner's creation witness, not an absent or invented revision")
        XCTAssertNil(request.expectedParentMessageID, "The new owner's frozen transcript is empty")
        XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, run)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
    }

    func testUnavailableSessionAndDefaultModelsBlockBothLocalAndAPISubmissions() async throws {
        for local in [false, true] {
            for inherited in [false, true] {
                let fixture = try await Fixture()
                defer { fixture.cleanUp() }
                let unavailableID = local ? MacLocalModelProfile.qwen35_08B.id : "disabled-api"
                let configurations = MacAppleChatConfigurationStore(applicationPort: fixture.port)
                try configurations.replaceEndpoints([MacRemoteEndpointConfiguration(id: "api", name: "API",
                    baseURL: "https://never-contact.invalid/v1", models: [
                        MacRemoteModelConfiguration(id: "fallback", modelName: "fallback", executionCapabilities: .init()),
                        MacRemoteModelConfiguration(id: "disabled-api", modelName: "disabled", isEnabled: false,
                            executionCapabilities: .init())
                    ])])
                if local {
                    let parameters = String(decoding: try JSONEncoder().encode(ModelParameters()), as: UTF8.self)
                    try fixture.port.upsertModelProfile(.init(id: unavailableID, displayName: "Disabled local",
                        provider: "local", modelName: unavailableID, runtime: "mlx", sourceJSON: "{}",
                        parametersJSON: parameters, selected: false, enabled: false, revision: 0,
                        createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
                }
                fixture.model.selectExecutionBackend(.fallback(id: "nola"), available: true)
                if inherited {
                    // A previously selected/synced default can reference a now
                    // disabled model. Persist that valid canonical reference;
                    // the send gate must not substitute the available fallback.
                    let value = try JSONSerialization.data(withJSONObject: ["version": 1,
                        "selection": ["kind": "profile", "profile_id": unavailableID]])
                    _ = try fixture.port.putProductSettingChecked(key: MacDefaultChatModelStore.key,
                        valueJSON: String(decoding: value, as: UTF8.self), expectedRevision: nil, updatedAtMs: 1)
                } else {
                    fixture.model.selectModel(MacChatModelOption(source: local ? .local : .configured,
                        modelID: local ? unavailableID : "disabled", displayName: "Disabled",
                        configurationID: local ? nil : unavailableID))
                }
                fixture.model.refreshModelConfiguration()
                XCTAssertEqual(fixture.model.modelSelectionResolution, .unavailable(profileID: unavailableID,
                    origin: inherited ? .default : .conversation, reason: .disabled))
                let prior = fixture.model.selectedMessages.count
                fixture.model.draft = "Must not use the available fallback"
                fixture.model.send()
                try await Task.sleep(for: .milliseconds(30))
                let dispatched = await fixture.runtime.runIDs()
                XCTAssertTrue(dispatched.isEmpty, "Unavailable local=\(local), inherited=\(inherited) dispatched a run")
                XCTAssertFalse(fixture.model.isRunning)
                XCTAssertEqual(fixture.model.selectedMessages.count, prior)
                XCTAssertEqual(fixture.model.draft, "Must not use the available fallback")
            }
        }
    }

    func testChangingDefaultKeepsActiveInvocationBytesAndNextInheritedRunUsesNewProfile() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let configurations = MacAppleChatConfigurationStore(applicationPort: fixture.port)
        try configurations.replaceEndpoints(["a", "b"].map { id in
            MacRemoteEndpointConfiguration(id: id, name: id, baseURL: "https://\(id).invalid/v1",
                models: [MacRemoteModelConfiguration(id: "profile:\(id)", modelName: "wire-\(id)",
                    executionCapabilities: .init())])
        })
        let preference = MacDefaultChatModelStore(application: fixture.port)
        let candidates = try MacChatModelCatalog.load(application: fixture.port).candidates
        let initial = try preference.set(.profile("profile:a"), expectedRevision: nil, candidates: candidates)
        fixture.model.selectExecutionBackend(.fallback(id: "nola"), available: true)
        fixture.model.refreshModelConfiguration()
        XCTAssertTrue(fixture.model.inheritsDefaultModel)
        let first = try await start(fixture)
        let firstRequest = await fixture.runtime.capturedRequest(first)
        let captured = try XCTUnwrap(firstRequest)
        XCTAssertEqual(captured.modelProfileID, "profile:a")
        let serializer = MacAgentHostRuntime(credentialStore: configurations)
        let before = try await serializer.initialRequestData(for: captured)

        _ = try preference.set(.profile("profile:b"), expectedRevision: initial.revision, candidates: candidates)
        fixture.model.refreshModelConfiguration()
        XCTAssertTrue(fixture.model.isRunning)
        XCTAssertTrue(fixture.model.inheritsDefaultModel, "Admission must not turn inheritance into an explicit selection")
        let unchanged = await fixture.runtime.capturedRequest(first)
        let after = try await serializer.initialRequestData(for: XCTUnwrap(unchanged))
        XCTAssertEqual(after, before, "Real Host invocation serialization must not reread a changed global default")

        try await fixture.runtime.commitCompletion(first)
        try await fixture.runtime.end(first, as: .eof)
        try await waitUntil { !fixture.model.isRunning && fixture.model.localRunRecovery == nil }
        let next = try await start(fixture, prompt: "Use the current inherited default")
        let nextRequest = await fixture.runtime.capturedRequest(next)
        let nextCaptured = try XCTUnwrap(nextRequest)
        XCTAssertEqual(nextCaptured.modelProfileID, "profile:b")
        XCTAssertEqual(nextCaptured.modelID, "wire-b")
        let nextData = try await serializer.initialRequestData(for: nextCaptured)
        let nextPayload = try XCTUnwrap(JSONSerialization.jsonObject(with: nextData) as? [String: Any])
        XCTAssertEqual((nextPayload["mac_launch"] as? [String: Any])?["model_profile_id"] as? String, "profile:b")
        try await fixture.runtime.commitCompletion(next)
        try await fixture.runtime.end(next, as: .eof)
        try await waitUntil { !fixture.model.isRunning && fixture.model.localRunRecovery == nil }
    }

    func testPostLaunchEOFAndErrorsRequireCanonicalTerminalAndLateCommitRecovers() async throws {
        for mode: TerminalContractLocalRuntime.StreamEnd in [.eof, .cancelled, .failed] {
            let fixture = try await Fixture()
            defer { fixture.cleanUp() }
            let run = try await start(fixture)
            try await fixture.runtime.end(run, as: mode)
            try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
            XCTAssertEqual(fixture.model.selectedConversationDisplayStatus, .uncertain,
                "Transport EOF without a committed terminal must not be presented as Ready")
            XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
            XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, run)
            XCTAssertTrue(fixture.activity.entries.isEmpty, "transport loss must not invent a completed or cancelled activity")
            fixture.model.draft = "Must not start a replacement yet"
            fixture.model.send()
            let ids = await fixture.runtime.runIDs()
            XCTAssertEqual(ids, [run])
            XCTAssertNotNil(fixture.model.localRunRecovery)
            await fixture.model.checkLocalRunRecovery()
            guard let inspection = fixture.model.localRecoveryInspection,
                  case .active = inspection.observation else {
                return XCTFail("Status check must read the exact persisted Run through Application")
            }
            XCTAssertNotNil(fixture.model.localRunRecovery, "a read-only check cannot invent terminal evidence")
            XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, run)
            let checkedIDs = await fixture.runtime.runIDs()
            XCTAssertEqual(checkedIDs, [run], "recovery check must not launch another transport")
            let terminalRevision = fixture.model.selectedTerminalTranscriptRevision
            try await fixture.runtime.commitCompletion(run)
            // Clearing recovery means canonical terminal was confirmed. The
            // sole owner must still drain presentation and record its outcome.
            try await waitUntil {
                fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                    && fixture.model.selectedTerminalTranscriptRevision > terminalRevision
                    && fixture.activity.entries.map(\.status) == [.answered]
            }
            XCTAssertEqual(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run)?.result.value.kind, .answered)
            XCTAssertTrue(fixture.model.selectedMessages.contains { $0.text == "Committed local answer" })
            XCTAssertEqual(fixture.activity.entries.map(\.status), [.answered])
        }
    }

    func testDisconnectedRecoveryDoesNotBlockAnotherConversationAndRestoresExactOwner() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let first = try await start(fixture)
        try await fixture.runtime.end(first, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        let original = try XCTUnwrap(fixture.model.localRunRecovery)
        fixture.model.newConversation()
        fixture.model.draft = "Independent task"
        XCTAssertTrue(fixture.model.selectedComposerState.canSend)
        XCTAssertNil(fixture.model.selectedComposerState.submissionBlockReason)
        XCTAssertNil(fixture.model.localRunRecovery)
        // Exercise the real configuration-write/admission ordering. Do not
        // refresh the revision or wait for an unrelated store read here:
        // dispatch must consume this edit's own commit receipt before Begin.
        fixture.model.selectExecutionBackend(.fallback(id: "codex"), available: true)
        XCTAssertTrue(fixture.model.selectedModelConfigurationPending)
        XCTAssertFalse(fixture.model.selectedComposerState.canSend)
        fixture.model.send()
        XCTAssertEqual(fixture.model.draft, "Independent task",
            "A configuration write in flight must not consume the unsent draft")
        XCTAssertFalse(fixture.model.isRunning)
        let blockedIDs = await fixture.runtime.runIDs()
        XCTAssertEqual(blockedIDs, [first], "Pending configuration must not dispatch a new Begin")
        try await fixture.model.waitForModelConfigurationCommit()
        XCTAssertFalse(fixture.model.selectedModelConfigurationPending)
        XCTAssertTrue(fixture.model.selectedComposerState.canSend)
        let second = try await start(fixture, prompt: "Independent task")
        XCTAssertNotEqual(first, second)
        try await fixture.runtime.commitCompletion(second)
        try await fixture.runtime.end(second, as: .eof)
        try await waitUntil { !fixture.model.isRunning }
        fixture.model.selectedConversationID = fixture.conversation
        await fixture.model.loadSelectedConversationIfNeeded()
        let restored = try XCTUnwrap(fixture.model.localRunRecovery)
        XCTAssertEqual(restored.expectedRunID, first)
        XCTAssertEqual(restored.canonicalAssistantMessageID, original.canonicalAssistantMessageID)
        XCTAssertGreaterThan(restored.generationID, original.generationID)
        XCTAssertEqual(fixture.model.selectedConversationDisplayStatus, .uncertain)
        fixture.model.draft = "Do not replace the unresolved run"
        XCTAssertFalse(fixture.model.selectedComposerState.canSend)
        fixture.model.send()
        let ids = await fixture.runtime.runIDs()
        XCTAssertEqual(ids, [first, second])
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: first))
    }

    func testMissingActivityAdvisoriesStillAllowsRecoveryReads() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        try fixture.executeSQLite("""
            UPDATE core_checkpoint_bodies SET checkpoint_json=json_remove(checkpoint_json,
                '$.state.execution.active_turn.engineering.recent_activity',
                '$.state.execution.active_turn.engineering.mutation_streak',
                '$.state.execution.active_turn.engineering.similar_mutation_streak',
                '$.state.execution.active_turn.engineering.last_mutation_sample');
            """)
        // This fixture changes the checkpoint body directly, so it is not a
        // journal-presentation frontier fixture. Exercise the exact recovery
        // reader, which must decode Core without fabricating a terminal.
        await fixture.model.checkLocalRunRecovery()
        XCTAssertNil(fixture.model.localRecoveryError)
        XCTAssertNotNil(fixture.model.localRecoveryInspection)
        XCTAssertEqual(fixture.model.localRunRecovery?.expectedRunID, run)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
    }

    func testPendingCancellationSurvivesConversationSwitchWithoutBlockingNewDraft() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        fixture.model.stop()
        try await waitUntil { !fixture.model.isDeliveringLocalCancellation }
        let intent = try XCTUnwrap(fixture.model.localCancellationRequest)
        fixture.model.newConversation()
        fixture.model.draft = "Another conversation"
        XCTAssertTrue(fixture.model.selectedComposerState.canSend)
        fixture.model.selectedConversationID = fixture.conversation
        XCTAssertEqual(fixture.model.localCancellationRequest, intent)
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertFalse(fixture.model.canResumeLocalRun)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
    }

    func testCommittedCompletionWinsLateCancellationError() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.commitCompletion(run)
        try await fixture.runtime.end(run, as: .cancelled)
        try await waitUntil {
            !fixture.model.isRunning && fixture.model.localRunRecovery == nil
                && fixture.activity.entries.map(\.status) == [.answered]
        }
        XCTAssertEqual(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run)?.result.value.kind, .answered)
        XCTAssertTrue(fixture.model.selectedMessages.contains { $0.text == "Committed local answer" })
        XCTAssertEqual(fixture.activity.entries.map(\.status), [.answered])
        let cancellationCount = await fixture.runtime.cancelCount()
        XCTAssertEqual(cancellationCount, 0)
    }

    func testOldRunEventCannotCompleteNextRun() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let old = try await start(fixture)
        try await fixture.runtime.commitCompletion(old)
        try await fixture.runtime.end(old, as: .eof)
        try await waitUntil {
            !fixture.model.isRunning && fixture.model.localRunRecovery == nil
                && fixture.activity.entries.map(\.status) == [.answered]
        }
        let next = try await start(fixture, prompt: "Start the next request")
        XCTAssertNotEqual(old, next)
        try await fixture.runtime.emitOldCompletion(into: next, oldRunID: old)
        try await fixture.runtime.end(next, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil }
        XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, next)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: next))
        try await fixture.runtime.commitCompletion(next, answer: "Next committed answer")
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.count == 2
        }
    }

    func testDefinitePreparationFailureRestoresUnsentInputWithoutCanonicalRun() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        await fixture.runtime.failPreparation()
        fixture.model.draft = "  Keep the original draft  "
        fixture.model.send()
        try await waitUntil { !fixture.model.isRunning }
        XCTAssertEqual(fixture.model.draft, "  Keep the original draft  ")
        XCTAssertNil(fixture.model.localRunRecovery)
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        XCTAssertNil(try fixture.port.activeRunIdentity(conversationID: fixture.conversation))
        let cancellationCount = await fixture.runtime.cancelCount()
        XCTAssertEqual(cancellationCount, 0)
    }

    func testTypedHostRejectionRollsBackOptimisticMessagesWithoutOverwritingNewDraft() async throws {
        for newDraft in ["", "A later unsent draft"] {
            let fixture = try await Fixture()
            defer { fixture.cleanUp() }
            await fixture.runtime.pauseBeforeAdmission()
            let before = fixture.model.selectedMessages.map(\.id)
            fixture.model.draft = "  Original rejected request  "
            fixture.model.send()
            try await waitUntil { await fixture.runtime.hasPendingAdmission() }
            XCTAssertTrue(fixture.model.selectedMessages.contains { $0.role == .user && $0.text.contains("Original rejected request") })
            if !newDraft.isEmpty { fixture.model.draft = newDraft }
            await fixture.runtime.finishPendingAdmission(rejected: true)
            try await waitUntil { !fixture.model.isRunning }
            XCTAssertEqual(fixture.model.selectedMessages.map(\.id), before)
            XCTAssertEqual(fixture.model.draft, newDraft.isEmpty ? "  Original rejected request  " : newDraft)
            XCTAssertNil(fixture.model.localRunRecovery)
            XCTAssertEqual(fixture.model.localSubmissionFailure?.conversationID, fixture.conversation)
            XCTAssertEqual(fixture.model.localSubmissionFailure?.code, "request_rejected_before_admission")
            XCTAssertNil(try fixture.port.activeRunIdentity(conversationID: fixture.conversation))
        }
    }

    func testUnknownTransportBeforeAckRetainsBubbleAndBlocksDuplicateSubmission() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        await fixture.runtime.pauseBeforeAdmission()
        fixture.model.draft = "May already have been admitted"
        fixture.model.send()
        try await waitUntil { await fixture.runtime.hasPendingAdmission() }
        let optimistic = fixture.model.selectedMessages.map(\.id)
        await fixture.runtime.finishPendingAdmission(rejected: false)
        try await waitUntil { fixture.model.localRunRecovery != nil }
        XCTAssertNil(fixture.model.localSubmissionFailure)
        XCTAssertEqual(fixture.model.selectedMessages.map(\.id), optimistic)
        XCTAssertEqual(fixture.model.draft, "")
        fixture.model.draft = "Do not duplicate"
        fixture.model.send()
        XCTAssertEqual(fixture.model.draft, "Do not duplicate")
        XCTAssertEqual(fixture.model.selectedMessages.map(\.id), optimistic)
    }

    func testUnknownRiskDecisionUsesFrozenOwnerAndResumesOnlyTheExistingRun() async throws {
        let service = UnknownRecoveryServiceFixture()
        let fixture = try await Fixture(unknownTransport: service.execute)
        defer { fixture.cleanUp() }
        let run = try await start(fixture, prompt: "Preserve this run")
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        let presentation = try XCTUnwrap(fixture.model.currentUnknownEffectRecovery)
        XCTAssertFalse(fixture.model.canResumeLocalRun)
        XCTAssertTrue(service.decisions.isEmpty, "inspection must never authorize retry")
        let decision = try presentation.decision(targetIndex: 0, reason: "I checked the external state")
        let delivery = Task { await fixture.model.resolveUnknownEffects(decision, presentation: presentation) }
        try await waitUntil { await fixture.runtime.resumeSnapshots().count == 1 }
        XCTAssertEqual(service.decisions, [decision])
        let snapshots = await fixture.runtime.resumeSnapshots()
        XCTAssertEqual(snapshots.map(\.runID), [run])
        let runs = await fixture.runtime.runIDs()
        XCTAssertEqual(runs, [run])
        try await fixture.runtime.commitCompletion(run)
        try await fixture.runtime.end(run, as: .eof)
        await delivery.value
    }

    func testStaleUnknownDecisionAfterConversationSwitchNeverReachesService() async throws {
        let service = UnknownRecoveryServiceFixture()
        let fixture = try await Fixture(unknownTransport: service.execute)
        defer { fixture.cleanUp() }
        let run = try await start(fixture, prompt: "First owner")
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        let presentation = try XCTUnwrap(fixture.model.currentUnknownEffectRecovery)
        let decision = try presentation.decision(targetIndex: 0, reason: "Explicit confirmation")
        fixture.model.newConversation()
        XCTAssertNotEqual(fixture.model.selectedConversationID, presentation.owner.conversationID)
        await fixture.model.resolveUnknownEffects(decision, presentation: presentation)
        XCTAssertTrue(service.decisions.isEmpty)
        let resumed = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(resumed.isEmpty)
    }

    func testInFlightRecoveryDoesNotMakeAnotherConversationBusy() async throws {
        let gate = DispatchSemaphore(value: 0)
        let service = UnknownRecoveryServiceFixture(resolutionGate: gate)
        let fixture = try await Fixture(unknownTransport: service.execute)
        defer { gate.signal(); fixture.cleanUp() }
        let run = try await start(fixture, prompt: "Recovery belongs to this conversation")
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        let presentation = try XCTUnwrap(fixture.model.currentUnknownEffectRecovery)
        let decision = try presentation.decision(targetIndex: 0, reason: "Explicit risk confirmation")
        let delivery = Task { await fixture.model.resolveUnknownEffects(decision, presentation: presentation) }
        try await waitUntil { service.hasStartedResolution }
        XCTAssertTrue(fixture.model.isResolvingUnknownEffects)
        fixture.model.newConversation()
        XCTAssertFalse(fixture.model.isResolvingUnknownEffects,
                       "A durable decision for another Run must not make this composer busy")
        XCTAssertFalse(fixture.model.isCheckingLocalRunRecovery)
        gate.signal()
        await delivery.value
        XCTAssertFalse(fixture.model.isResolvingUnknownEffects)
        XCTAssertNil(fixture.model.currentUnknownEffectRecovery)
        let resumed = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(resumed.isEmpty, "Detached reply must not resume whichever conversation is selected now")
    }

    func testExplicitResumeRetainsRunMessagesAndOriginalRequestWhenComposerDefaultsChange() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture, prompt: "Original request only once")
        let captured = await fixture.runtime.requestSnapshot(run)
        let original = try XCTUnwrap(captured)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        let before = try fixture.port.loadConversationWindow(fixture.conversation).projection.messages.map(\.messageID)
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canResumeLocalRun)
        fixture.model.style = .detailed
        fixture.model.draft = "A different unsent follow-up"
        let resume = Task { await fixture.model.resumeLocalRun() }
        try await waitUntil { await fixture.runtime.resumeSnapshots().count == 1 }
        let resumed = await fixture.runtime.resumeSnapshots()
        XCTAssertEqual(resumed, [original], "resume has no authority to replace the original prompt or model settings")
        let runIDs = await fixture.runtime.runIDs()
        XCTAssertEqual(runIDs, [run])
        XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, run)
        XCTAssertEqual(try fixture.port.loadConversationWindow(fixture.conversation).projection.messages.map(\.messageID), before)
        XCTAssertEqual(fixture.model.draft, "A different unsent follow-up")
        try await fixture.runtime.commitCompletion(run)
        try await fixture.runtime.end(run, as: .eof)
        await resume.value
        try await waitUntil {
            !fixture.model.isRunning && fixture.model.localRunRecovery == nil
                && fixture.activity.entries.map(\.status) == [.answered]
        }
        let messages = try fixture.port.loadConversationWindow(fixture.conversation).projection.messages
        XCTAssertEqual(messages.map(\.messageID), before)
        XCTAssertEqual(messages.filter { $0.role == .user }.count, 1)
    }

    func testResumeTransportFailureKeepsSameRunPendingWithoutCompletionActivity() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .failed)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.runtime.failNextResume()
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canResumeLocalRun)
        await fixture.model.resumeLocalRun()
        try await waitUntil { !fixture.model.isResumingLocalRun && fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        let resumed = await fixture.runtime.resumeSnapshots()
        XCTAssertEqual(resumed.map(\.runID), [run])
        XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, run)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        try await fixture.runtime.commitCompletion(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.answered]
        }
    }

    func testRevokedFrozenWorkspacePreventsResumeTransport() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canResumeLocalRun)
        fixture.bookmarks.removeWorkspace(conversationID: fixture.conversation)
        await fixture.model.resumeLocalRun()
        let resumed = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(resumed.isEmpty, "revoked original authority cannot be replaced by a sandbox or later default")
        XCTAssertNotNil(fixture.model.localRunRecovery)
        XCTAssertFalse(fixture.model.isRunning)
        XCTAssertNotNil(fixture.model.localRecoveryError)
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, run)
        try await fixture.runtime.commitCompletion(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.answered]
        }
    }

    func testCanonicalTerminalDuringAwaitedRevalidationPreventsOldRunDispatch() async throws {
        let fixture = try await Fixture()
        defer { fixture.resolutionGate.release(); fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canResumeLocalRun)
        let bookmarks = fixture.bookmarks
        let gate = fixture.resolutionGate
        await fixture.runtime.setBeforeRevalidate {
            // Initial admission already passed. Force the second, launch-bound
            // authorization check through real off-actor bookmark resolution.
            bookmarks.close()
            gate.arm()
        }
        let resumeTask = Task { await fixture.model.resumeLocalRun() }
        try await waitUntil { gate.isEntered }
        let beforeDispatch = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(beforeDispatch.isEmpty)
        try await fixture.runtime.commitCompletion(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.answered]
        }
        gate.release()
        // The resumed validation must unwind; no stale transport may replace
        // the already-finalized canonical Run after this await returns.
        try await waitUntil { await fixture.runtime.finishedResumeAttempts() == 1 }
        await resumeTask.value
        let afterDispatch = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(afterDispatch.isEmpty)
        XCTAssertEqual(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run)?.result.value.kind, .answered)
        XCTAssertNil(fixture.model.localRunRecovery)
    }

    func testStopBeforeResumeTransportReadyRetainsIntentWithoutSyntheticTerminal() async throws {
        let fixture = try await Fixture()
        defer { Task { await fixture.runtime.releaseReady() }; fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canResumeLocalRun)
        await fixture.runtime.pauseNextReady()
        let resumeTask = Task { await fixture.model.resumeLocalRun() }
        try await waitUntil { await fixture.runtime.isWaitingForReady() }
        let terminalRevision = fixture.model.selectedTerminalTranscriptRevision
        fixture.model.stop()
        let beforeReady = await fixture.runtime.cancelCount()
        XCTAssertEqual(beforeReady, 0, "No cancel may be delivered to an unready transport")
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertNotNil(fixture.model.localRunRecovery)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        XCTAssertEqual(fixture.model.selectedTerminalTranscriptRevision, terminalRevision)
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        let intent = try XCTUnwrap(fixture.model.localCancellationRequest)
        let durable = try await fixture.runtime.cancellationFacts(run)
        XCTAssertEqual(durable.status, "cancelling")
        XCTAssertEqual(durable.pendingCancels, 1)
        await fixture.runtime.releaseReady()
        try await waitUntil { await fixture.runtime.cancelCount() == 1 }
        let delivered = await fixture.runtime.deliveredCancellationRequests()
        XCTAssertEqual(delivered, [intent], "Ready resumes deliver the persisted exact cancellation request")
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        try await fixture.runtime.end(run, as: .cancelled)
        await resumeTask.value
        try await waitUntil { !fixture.model.isRunning && fixture.model.localRunRecovery != nil }
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertEqual(fixture.model.selectedTerminalTranscriptRevision, terminalRevision)
        XCTAssertEqual(fixture.model.localCancellationRequest, intent)
        try await fixture.runtime.commitCancellation(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.cancelled]
        }
    }

    func testCancellationReceiptFailureInterruptsOnlyOwnedResumeAndRetainsRetryableIntent() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        let resumeTask = Task { await fixture.model.resumeLocalRun() }
        try await waitUntil { await fixture.runtime.resumeSnapshots().count == 1 }
        let messages = fixture.model.selectedMessages.map(\.id)
        let revision = fixture.model.selectedTerminalTranscriptRevision
        let wrongIdentityInterrupted = await fixture.runtime.interruptOwnedTransport(
            conversationID: fixture.conversation, expectedRunID: run, assistantMessageID: "wrong-assistant")
        XCTAssertFalse(wrongIdentityInterrupted)
        // Fail only the durable cancellation command receipt, not reads,
        // projection subscriptions or unrelated Application transactions.
        try fixture.executeSQLite("""
            CREATE TRIGGER reject_test_cancellation BEFORE INSERT ON command_receipts
            WHEN NEW.command_type = 'request_run_cancellation'
            BEGIN SELECT RAISE(ABORT, 'isolated cancellation receipt fault'); END;
            """)
        fixture.model.stop()
        let intent = try XCTUnwrap(fixture.model.localCancellationRequest)
        try await waitUntil { await fixture.runtime.interruptedRunIDs() == [run] }
        await resumeTask.value
        try await waitUntil { !fixture.model.isRunning && fixture.model.canRetryLocalRunCancellation }
        XCTAssertEqual(fixture.model.localCancellationRequest, intent)
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertNotNil(fixture.model.localRecoveryError)
        XCTAssertNotNil(fixture.model.localRunRecovery)
        XCTAssertEqual(fixture.model.selectedTerminalTranscriptRevision, revision)
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        let beforeRetry = await fixture.runtime.deliveredCancellationRequests()
        XCTAssertTrue(beforeRetry.isEmpty, "Storage rejection cannot dispatch a durable cancellation")
        let facts = try await fixture.runtime.cancellationFacts(run)
        XCTAssertNotEqual(facts.status, "cancelling", "Receipt rejection rolls back the cancellation transaction")
        XCTAssertEqual(facts.pendingCancels, 0)
        try fixture.executeSQLite("DROP TRIGGER reject_test_cancellation;")
        await fixture.model.retryLocalRunCancellation()
        let delivered = await fixture.runtime.deliveredCancellationRequests()
        XCTAssertEqual(delivered, [intent])
        XCTAssertEqual(fixture.model.selectedMessages.map(\.id), messages)
        let runs = await fixture.runtime.runIDs()
        XCTAssertEqual(runs, [run])
        let persisted = try await fixture.runtime.cancellationFacts(run)
        XCTAssertEqual(persisted.status, "cancelling")
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        try await fixture.runtime.commitCancellation(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.cancelled]
        }
    }

    func testFailedCancellationDeliveryRetriesSameIntentWithoutWorkspaceAuthorizationOrNewTurn() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        let originalMessages = fixture.model.selectedMessages.map(\.id)
        let terminalRevision = fixture.model.selectedTerminalTranscriptRevision
        await fixture.runtime.failNextCancellationDelivery()
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canCancelLocalRecovery)
        fixture.model.stop()
        try await waitUntil {
            await fixture.runtime.deliveredCancellationRequests().count == 1
                && !fixture.model.isDeliveringLocalCancellation
        }
        let intent = try XCTUnwrap(fixture.model.localCancellationRequest)
        XCTAssertTrue(fixture.model.canRetryLocalRunCancellation)
        XCTAssertFalse(fixture.model.canCancelLocalRecovery, "Pending cancellation must not offer a new intent")
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertNotNil(fixture.model.localRecoveryError)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        fixture.bookmarks.removeWorkspace(conversationID: fixture.conversation)
        await fixture.model.retryLocalRunCancellation()
        let attempts = await fixture.runtime.deliveredCancellationRequests()
        XCTAssertEqual(attempts, [intent, intent], "Retry preserves cancellation identity and requested timestamp")
        let successfulDeliveries = await fixture.runtime.cancelCount()
        XCTAssertEqual(successfulDeliveries, 1)
        let runs = await fixture.runtime.runIDs()
        XCTAssertEqual(runs, [run])
        let resumes = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(resumes.isEmpty, "Cancellation-only delivery must not resume model execution")
        XCTAssertEqual(fixture.model.selectedMessages.map(\.id), originalMessages)
        XCTAssertEqual(fixture.model.localCancellationRequest, intent)
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertNotNil(fixture.model.localRunRecovery)
        XCTAssertEqual(fixture.model.selectedTerminalTranscriptRevision, terminalRevision)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        try await fixture.runtime.commitCancellation(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.cancelled]
        }
    }

    func testStopDuringSecondBookmarkValidationPersistsIntentWithoutLaunchingResume() async throws {
        let fixture = try await Fixture()
        defer { fixture.resolutionGate.release(); fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canResumeLocalRun)
        let gate = fixture.resolutionGate
        let bookmarks = fixture.bookmarks
        await fixture.runtime.setBeforeRevalidate { bookmarks.close(); gate.arm() }
        let resumeTask = Task { await fixture.model.resumeLocalRun() }
        try await waitUntil { gate.isEntered }
        let terminalRevision = fixture.model.selectedTerminalTranscriptRevision
        fixture.model.stop()
        let intent = try XCTUnwrap(fixture.model.localCancellationRequest)
        XCTAssertEqual(intent.expectedRunID, run)
        let durable = try await fixture.runtime.cancellationFacts(run)
        XCTAssertEqual(durable.status, "cancelling")
        XCTAssertEqual(durable.pendingCancels, 1)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        // Revoking access after cancellation must not retract its committed
        // intent or make directory reauthorization necessary for read-only checks.
        fixture.bookmarks.removeWorkspace(conversationID: fixture.conversation)
        gate.release()
        await resumeTask.value
        let dispatched = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(dispatched.isEmpty)
        let cancellationDeliveries = await fixture.runtime.cancelCount()
        XCTAssertEqual(cancellationDeliveries, 0, "no Host transport was launched")
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertNotNil(fixture.model.localRunRecovery)
        XCTAssertEqual(fixture.model.selectedTerminalTranscriptRevision, terminalRevision)
        XCTAssertEqual(fixture.model.localCancellationRequest, intent)
        fixture.model.stop()
        await fixture.model.checkLocalRunRecovery()
        let repeated = try await fixture.runtime.cancellationFacts(run)
        XCTAssertEqual(repeated.revision, durable.revision)
        XCTAssertEqual(fixture.model.localCancellationRequest, intent)
        XCTAssertFalse(fixture.model.canResumeLocalRun)
        try await fixture.runtime.commitCancellation(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.cancelled]
        }
    }
}
