// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

@MainActor
final class IOSGitHubSkillTests: XCTestCase {
    func testInstalledPackagesRequireActivationAndHonorCurrentEnablement() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-skill-fixture-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.db"))
        let commit = String(repeating: "a", count: 40)
        let reference = "nola://github-skills/owner/repo/tree/\(commit)/skills/card"
        let source = try XCTUnwrap(IOSGitHubSkillPackages.sourceURL(reference: reference))
        let directory = root.appendingPathComponent("nola.db.skill-packages/owner/repo/\(commit)/skills/card/.package/skills/card")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try "# Card".write(to: directory.appendingPathComponent("SKILL.md"), atomically: true, encoding: .utf8)
        try "Expected content".write(to: directory.appendingPathComponent("sample.md"), atomically: true, encoding: .utf8)
        let package = AgentCoreJSONValue.object(["status": .string("prepared"), "id": .string("card"),
            "version": .string(commit), "content": .string("# Card"), "description": .string("Card fixture"),
            "instructions_ref": .string(reference), "source_url": .string(source.absoluteString)])
        let packages = IOSGitHubSkillPackages.shared
        _ = try await packages.register(package, runtime: runtime, permit: nil)
        _ = try await packages.register(package, runtime: runtime, permit: nil)
        let settings = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let items = try settings.loadCapabilityLibrary().uploadedSkills.filter { $0.name == "card" }
        XCTAssertEqual(items.count, 1)
        let item = try XCTUnwrap(items.first)
        let executor = IOSAgentHostExecutor(applicationRuntime: runtime)
        let session = UUID().uuidString
        await executor.installSkillSnapshot([.init(id: item.id.uuidString, name: item.name,
            summary: item.description, instructions: item.content, version: item.version, requiredTools: [],
            validationSteps: [], packageReference: item.packageReference)], sessionId: session)
        func read() async -> IOSAgentHostCallResult {
            await executor.executeBridge(callId: UUID().uuidString, toolName: "device_skill_read_resource", capability: .none,
                arguments: .object(["skill": .string("card"), "path": .string("sample.md")]), grant: nil, sessionId: session)
        }
        let inactive = await read()
        XCTAssertTrue(inactive.isError)
        let invocation = await executor.executeBridge(callId: UUID().uuidString, toolName: "skill_invoke", capability: .none,
            arguments: .object(["skill": .string("card")]), grant: nil, sessionId: session)
        XCTAssertFalse(invocation.isError, "\(invocation.output)")
        let content = await read()
        XCTAssertEqual(content.output.objectFields?["content"], .string("Expected content"))
        _ = try settings.setSkillEnabled(false, expected: settings.loadSkillEditor(id: item.id))
        let disabled = await read()
        XCTAssertTrue(disabled.isError)
    }

    func testRealChatInstallsPackageThenReadsSharedFileInFreshConversation() async throws {
        let env = ProcessInfo.processInfo.environment
        guard env["NOLA_REAL_LLM_E2E"] == "1" else { throw XCTSkip("Requires a real Responses endpoint") }
        let ownRoot = env["NOLA_GITHUB_SKILL_UI_FIXTURE_ROOT"] == nil
        let root: URL
        if let path = env["NOLA_GITHUB_SKILL_UI_FIXTURE_ROOT"] {
            root = URL(fileURLWithPath: path).standardizedFileURL
            let prefix = "nola-ios-github-skill-"
            guard ["private/tmp", "tmp"].contains(root.deletingLastPathComponent().path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))),
                  root.lastPathComponent.hasPrefix(prefix),
                  UUID(uuidString: String(root.lastPathComponent.dropFirst(prefix.count))) != nil,
                  !FileManager.default.fileExists(atPath: root.appendingPathComponent("nola.db").path) else {
                throw IOSApplicationRuntimeError.rejected("Expected a fresh isolated GitHub Skill UI fixture")
            }
        } else {
            root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-github-skill-\(UUID())")
        }
        defer { if ownRoot { try? FileManager.default.removeItem(at: root) } }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.db"))
        let previous = IOSApplicationRuntime.shared
        IOSApplicationRuntime.installShared(runtime)
        defer { IOSApplicationRuntime.installShared(previous) }
        let model = try XCTUnwrap(env["NOLA_REAL_LLM_MODEL"])
        let configured = ModelInfo(id: UUID().uuidString, name: model, source: "cloud", isLocal: false,
            baseURL: try XCTUnwrap(env["NOLA_REAL_LLM_ENDPOINT"]), apiKey: try XCTUnwrap(env["NOLA_REAL_LLM_API_KEY"]),
            upstreamModelId: model, wireProtocol: .responses, executionCapabilities: .init(toolCalls: true))
        try ModelSelectionManager.addConfiguredCloudModel(configured)
        let published = try XCTUnwrap(ModelSelectionManager.availableModels(using: runtime).first { $0.id == configured.id })
        defer { if let account = published.cloudCredential?.account { IOSCloudCredentialOwner.shared.delete(account) } }

        func send(_ message: String) async throws -> (ChatManager, String) {
            let store = try LocalConversationStore(applicationRuntime: runtime)
            let conversation = try store.createConversation(title: "GitHub Skill", language: "en")
            let container = ServiceContainer(applicationRuntime: runtime)
            container.conversationStore = store
            container.contextManager = DefaultContextManager(store: store, profileStore: CanonicalChatMemoryProfileStore(applicationRuntime: runtime))
            let manager = ChatManager(container: container, restorePersistedState: false)
            manager.currentConversationId = conversation.id
            manager.selectModel(published, persistAsDefault: false)
            try manager.persistSelectedModelForCurrentConversation(published)
            let approvals = Task { @MainActor in
                var answered = Set<String>()
                while !Task.isCancelled {
                    if let prompt = manager.localAgentPermissionRequest, answered.insert(prompt.requestId).inserted {
                        manager.resolveLocalAgentPermission(requestId: prompt.requestId,
                            ["device_github_skill_find", "device_github_skill_install", "device_skill_read_resource", "skill_invoke"].contains(prompt.toolId) ? .allowOnce : .deny)
                    }
                    try? await Task.sleep(nanoseconds: 50_000_000)
                }
            }
            defer { approvals.cancel() }
            return (manager, try await manager.processMessage(message))
        }

        let (installer, installAnswer) = try await send("在 GitHub 上找到 Yuan1z0825 的 nature-skills，安装其中的 nature-paper-card 论文精读卡 Skill 到这台 iPhone，启用并保存，供新对话使用。")
        XCTAssertTrue(installer.hostToolEvents.contains { $0.name == "device_github_skill_find" && $0.status == .succeeded }, installAnswer)
        XCTAssertTrue(installer.hostToolEvents.contains { $0.name == "device_github_skill_install" && $0.status == .succeeded }, installAnswer)
        let coldRuntime = try IOSApplicationRuntime(databaseURL: runtime.databaseURL)
        let store = CanonicalChatMemoryProfileStore(applicationRuntime: coldRuntime)
        let item = try XCTUnwrap(store.loadCapabilityLibrary().uploadedSkills.first { $0.name == "nature-paper-card" })
        XCTAssertNotNil(IOSGitHubSkillPackages.sourceURL(reference: item.packageReference))
        let first = try store.loadSkillEditor(id: item.id)
        let disabled = try store.setSkillEnabled(false, expected: first)
        XCTAssertFalse(disabled.record.enabled)
        _ = try store.setSkillEnabled(true, expected: disabled)
        let (manager, result) = try await send("调用刚安装的 nature-paper-card。请实际读取其 manifest.yaml 指定的共享术语表，回复它的英文标题。")
        XCTAssertTrue(result.localizedCaseInsensitiveContains("terminology ledger"), result)
        XCTAssertTrue(manager.hostToolEvents.contains { $0.name == "device_skill_read_resource" && $0.status == .succeeded })
    }

    func testPairedHostInstallsSkillInItsOwnSettingsFromNativeChatStream() async throws {
        let env = ProcessInfo.processInfo.environment
        guard env["NOLA_REAL_GITHUB_SKILL_HOST_E2E"] == "1" else {
            throw XCTSkip("Requires an isolated paired Nola Host with a real model")
        }
        let url = try XCTUnwrap(env["NOLA_REAL_WEB_AGENT_HOST_URL"].flatMap(URL.init(string:)))
        let token = try XCTUnwrap(env["NOLA_REAL_WEB_AGENT_HOST_TOKEN"])
        let before = try await WebAgentProvider.fetchHostAgentSettings(baseURL: url, token: token)
        let hostID = try XCTUnwrap(before.host_identity?.id)
        let clientID = "ios-skill-paired"
        let conversationID = UUID().uuidString
        let runID = UUID().uuidString
        XCTAssertFalse(before.uploaded_skills.contains { $0.name == "nature-paper-card" })
        let provider = WebAgentProvider(config: .init(baseURL: url, token: token, expectedHostId: hostID, clientId: clientID))
        let input = ModelInput(dateRange: .init(start: "2026-11-20T00:00:00Z", end: "2026-11-20T00:01:00Z"),
            candidates: [], userPrefs: [
                "message": "请在这个宿主上安装并启用 https://github.com/Yuan1z0825/nature-skills/tree/main/skills/nature-paper-card，让我的新会话也能使用。",
                "host_conversation_id": conversationID, "submission_id": runID, "user_message_id": UUID().uuidString,
                "assistant_message_id": "\(runID):assistant", "execution_backend": "nola"
            ])
        let journal = IOSCommittedJournalTestReader()
        var completed: [String] = []
        for try await event in provider.generateAgentEventStream(input: input) {
            try journal.observe(event)
            switch event {
            case .trace(let trace) where trace.kind == .toolCompleted:
                if let name = trace.payload.toolName { completed.append(name) }
            case .permissionRequest(let request, _), .delegatedPermissionRequest(let request):
                let allowed = request.hostId == hostID && request.clientId == clientID
                    && request.sessionId == conversationID
                    && ["github_skill_find", "github_skill_install"].contains(request.toolName)
                try await provider.submitHostPermissionDecision(
                    ChatManager.hostPermissionDecisionPayload(for: request, decision: allowed ? .allowOnce : .deny))
                XCTAssertTrue(allowed, "Unexpected paired Host approval: \(request.toolName)")
            case .error(let message): XCTFail(message)
            default: break
            }
        }
        let answer = try await journal.text(provider: provider)
        XCTAssertTrue(completed.contains("github_skill_install"), "Host tools: \(completed); answer: \(answer)")
        let after = try await WebAgentProvider.fetchHostAgentSettings(baseURL: url, token: token)
        let saved = try XCTUnwrap(after.uploaded_skills.first { $0.name == "nature-paper-card" })
        XCTAssertEqual(saved.enabled, true)
        XCTAssertNotNil(IOSGitHubSkillPackages.sourceURL(reference: saved.packageReference))
        XCTAssertFalse(saved.content.isEmpty)
    }
}
