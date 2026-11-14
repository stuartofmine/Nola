// SPDX-License-Identifier: Apache-2.0
import AppKit
import Combine
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacExpertRoleConfigurationTests: XCTestCase {
    func testEffortSelectionPreservesOpaqueDefaultsAndPerModelOverrides() throws {
        let original: MacCanonicalJSONValue = .object([
            "modelSelectionID": .string("configured:model"), "expertReasoningEffort": .string("high"),
            "modelOverrides": .object(["configured:model": .object(["temperature": .number(0.2), "future": .bool(true)])]),
            "futureDefault": .array([.string("keep"), .null])
        ])
        var draft = try AppleExpertConfigurationDraft(defaults: original)
        XCTAssertEqual(draft.reasoningEffort, "high")
        XCTAssertEqual(try draft.applying(to: original), original)
        draft.reasoningEffort = "max"
        guard case .object(let changed) = try draft.applying(to: original), case .object(let old) = original else {
            return XCTFail("Expected defaults")
        }
        XCTAssertEqual(changed["expertReasoningEffort"], .string("max"))
        XCTAssertEqual(changed["modelOverrides"], old["modelOverrides"])
        XCTAssertEqual(changed["futureDefault"], old["futureDefault"])
        draft.reasoningEffort = "automatic"
        guard case .object(let inherited) = try draft.applying(to: original) else { return XCTFail("Expected defaults") }
        XCTAssertNil(inherited["expertReasoningEffort"])
        XCTAssertEqual(inherited["modelOverrides"], old["modelOverrides"])
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: .object([:])).reasoningEffort, "automatic")
        XCTAssertThrowsError(try AppleExpertConfigurationDraft(defaults: .object(["expertReasoningEffort": .bool(false)])))
    }

    func testCoreRoleCatalogContainsReadablePromptsToolPacksAndRealSkillIdentities() throws {
        let root = temporaryRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("roles.sqlite"))
        let catalog = try MacAgentRoomService(runtime: port).roleTemplates()
        XCTAssertEqual(catalog.version, 1)
        XCTAssertEqual(catalog.templates.count, 7)
        XCTAssertTrue(catalog.features.reasoningEffort)
        XCTAssertTrue(catalog.features.explicitGenerationReasoning)
        XCTAssertTrue(catalog.features.rejoin)
        let descriptors = MacBuiltInSkillRegistry.descriptors(overridesRootURL: root.appendingPathComponent("no-overrides"))
        for role in catalog.templates {
            XCTAssertNotNil(UUID(uuidString: role.id))
            XCTAssertFalse(role.prompt.isEmpty)
            XCTAssertFalse(role.toolPack.tools.isEmpty)
            XCTAssertFalse(role.localizedName(locale: Locale(identifier: "zh_Hans_CN")).isEmpty)
            for skill in role.skills {
                XCTAssertNotNil(MacBuiltInSkillID(rawValue: skill.id), "Role must reference a registered real bundle Skill")
                let resource = try XCTUnwrap(descriptors.first { $0.id.rawValue == skill.id })
                XCTAssertFalse(try String(contentsOf: resource.instructionURL, encoding: .utf8).isEmpty)
                XCTAssertFalse(skill.instructions.isEmpty)
            }
        }
        let data = try JSONEncoder().encode(catalog)
        var document = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        document["version"] = 2
        XCTAssertThrowsError(try JSONDecoder().decode(AppleExpertRoleTemplateCatalog.self,
            from: JSONSerialization.data(withJSONObject: document)))
        document["version"] = 1
        if var templates = document["templates"] as? [[String: Any]], var duplicate = templates.first {
            duplicate["id"] = catalog.templates[0].id.uppercased()
            templates.append(duplicate)
            document["templates"] = templates
            XCTAssertThrowsError(try JSONDecoder().decode(AppleExpertRoleTemplateCatalog.self,
                from: JSONSerialization.data(withJSONObject: document)))
            document["templates"] = Array(templates.dropLast())
        }
        document.removeValue(forKey: "features")
        let legacy = try JSONDecoder().decode(AppleExpertRoleTemplateCatalog.self,
            from: JSONSerialization.data(withJSONObject: document))
        XCTAssertFalse(legacy.features.reasoningEffort)
        XCTAssertFalse(legacy.features.rejoin)
    }

    func testEffortOptionsUseOnlyDeclaredSelectedOrInheritedModel() throws {
        let root = temporaryRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("models.sqlite"))
        var source: [String: Any] = ["schemaVersion": 1, "endpointID": "fixture", "endpointName": "Fixture",
            "baseURL": "https://fixture.invalid/v1", "endpointOrder": 0, "endpointEnabled": true,
            "modelEnabled": true, "modelOrder": 0, "priority": 100, "capabilities": ["chat"],
            "wireProtocol": "responses", "credentialAccount": "fixture-account",
            "reasoning": ["control": "effort", "efforts": ["low", "medium", "high", "max"]]]
        for id in ["declared", "unknown"] {
            if id == "unknown" { source.removeValue(forKey: "reasoning") }
            try port.upsertModelProfile(.init(id: id, displayName: id, provider: "openai-compatible", modelName: id,
                runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
                parametersJSON: "{}", selected: false, enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
        }
        let repository = MacCanonicalConversationRepository(store: port)
        XCTAssertEqual(try repository.agentReasoningEfforts(modelSelectionID: "configured:declared"), ["low", "medium", "high", "max"])
        XCTAssertEqual(try repository.agentReasoningEfforts(modelSelectionID: "configured:unknown"), [])
        XCTAssertEqual(try repository.agentReasoningEfforts(modelSelectionID: "configured:missing"), [])
        let preference = MacDefaultChatModelPresentation(store: .init(application: port))
        preference.select(.profile("declared"))
        XCTAssertNil(preference.error)
        XCTAssertEqual(try repository.agentReasoningEfforts(modelSelectionID: ""), ["low", "medium", "high", "max"])
        preference.select(.profile("unknown"))
        XCTAssertEqual(try repository.agentReasoningEfforts(modelSelectionID: ""), [])
    }

    func testBlankComposerMentionValidatesExactRevisionWithoutCreatingRoom() async throws {
        let root = temporaryRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("mentions.sqlite"))
        let service = MacAgentRoomService(runtime: port)
        let directory = try service.createAgent(name: "Writer", description: "Code", defaults: .object([:]), directory: service.directory())
        let agent = try XCTUnwrap(directory.agents.first)
        let workspace = MacAgentRoomWorkspace(service: service)
        try await workspace.prepareExpertMention(agentID: agent.id, expectedRevision: agent.revision,
            conversationID: "unsaved-composer", isPersisted: false)
        XCTAssertTrue(try service.list().isEmpty)
        _ = try service.updateAgent(agent, name: "Writer edited", description: "New configuration",
            availability: "active", directory: directory, defaults: agent.defaults)
        do {
            try await workspace.prepareExpertMention(agentID: agent.id, expectedRevision: agent.revision,
                conversationID: "unsaved-composer", isPersisted: false)
            XCTFail("A stale observed candidate must not silently upgrade")
        } catch { }
        XCTAssertTrue(try service.list().isEmpty)
    }

    func testExistingConversationRequiresExplicitReintroductionAndRetainsOldContext() async throws {
        let root = temporaryRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("rejoin.sqlite"))
        let source: [String: Any] = ["schemaVersion": 1, "endpointID": "fixture", "endpointName": "Fixture",
            "baseURL": "https://fixture.invalid/v1", "endpointOrder": 0, "endpointEnabled": true,
            "modelEnabled": true, "modelOrder": 0, "priority": 100, "capabilities": ["chat"],
            "wireProtocol": "responses", "credentialAccount": "fixture-account",
            "reasoning": ["control": "effort", "efforts": ["low", "high"]]]
        try port.upsertModelProfile(.init(id: "declared", displayName: "Declared", provider: "openai-compatible", modelName: "declared",
            runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: "{}", selected: false, enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
        let repository = MacCanonicalConversationRepository(store: port)
        let conversation = MacConversation(title: "Existing", preview: "", updatedAt: .now, isPinned: false)
        _ = try repository.ensureConversation(conversation)
        let service = MacAgentRoomService(runtime: port)
        let directory = try service.createAgent(name: "Writer", description: "Code",
            defaults: .object(["modelSelectionID": .string("configured:declared"), "expertReasoningEffort": .string("low")]),
            directory: service.directory())
        let agent = try XCTUnwrap(directory.agents.first)
        let association = try service.ensureForConversation(conversation.id)
        let joined = try service.join(agent: agent, room: association.room)
        let previous = try XCTUnwrap(joined.room.participants.first)
        let oldSettings = try port.loadConversationMetadata(previous.contextConversationID)?.settingsJSON
        let updated = try service.updateAgent(agent, name: "Writer", description: "Code", availability: "active", directory: directory,
            defaults: .object(["modelSelectionID": .string("configured:declared"), "expertReasoningEffort": .string("high")]))
        let latest = try XCTUnwrap(updated.agents.first)
        let workspace = MacAgentRoomWorkspace(service: service)
        do {
            try await workspace.prepareExpertMention(agentID: latest.id, expectedRevision: latest.revision,
                conversationID: conversation.id, isPersisted: true)
            XCTFail("An updated member must not silently replace its previous context")
        } catch MacExpertMentionPreparationError.reintroductionRequired { }
        let unchanged = try XCTUnwrap(service.forConversation(conversation.id))
        XCTAssertEqual(unchanged.room.room.participants.count, 1)
        try await workspace.prepareExpertMention(agentID: latest.id, expectedRevision: latest.revision,
            conversationID: conversation.id, isPersisted: true, allowReintroduction: true)
        let refreshed = try XCTUnwrap(service.forConversation(conversation.id))
        XCTAssertEqual(refreshed.room.room.participants.count, 1)
        XCTAssertNil(refreshed.room.room.participants.first { $0.id == previous.id })
        let active = try XCTUnwrap(refreshed.room.room.participants.first { $0.active })
        XCTAssertEqual(active.agentRevision, latest.revision)
        XCTAssertNotEqual(active.contextConversationID, previous.contextConversationID)
        XCTAssertEqual(try port.loadConversationMetadata(previous.contextConversationID)?.settingsJSON, oldSettings)
    }

    func testBuiltinLibraryBrowsesWithoutWritesAndCreatesEditableRoleCopy() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let fixture = try nativeFixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let before = try fixture.service.directory()
        let catalog = try fixture.service.roleTemplates()
        let workspace = MacAgentRoomWorkspace(service: fixture.service)
        workspace.agentModelOptions = { try fixture.repository.agentModelOptions() }
        workspace.agentReasoningEfforts = { try fixture.repository.agentReasoningEfforts(modelSelectionID: $0) }
        let host = NSHostingView(rootView: MacExpertDirectoryManagementView(workspace: workspace)
            .environment(\.accessibilityEnabled, true))
        let window = makeWindow(host, width: 980, height: 780)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        let role = try XCTUnwrap(catalog.templates.first)
        try await wait { self.element(host, "mac-expert-builtin-" + role.id) != nil }
        XCTAssertEqual(catalog.templates.count, 7)
        for builtin in catalog.templates {
            _ = element(host, "mac-expert-builtin-" + builtin.id)?.accessibilityPerformPress?()
            try await wait { self.element(host, "mac-expert-builtin-preview") != nil }
        }
        XCTAssertEqual(try fixture.service.directory().agents.count, before.agents.count,
            "Browsing built-in roles must not create persistent agents")
        _ = element(host, "mac-expert-builtin-" + role.id)?.accessibilityPerformPress?()
        try await Task.sleep(for: .milliseconds(100))
        _ = element(host, "mac-expert-customize-builtin")?.accessibilityPerformPress?()
        try await wait { self.element(host, "mac-expert-save") != nil }
        _ = element(host, "mac-expert-save")?.accessibilityPerformPress?()
        try await wait { (try? fixture.service.directory().agents.count) == before.agents.count + 1 }
        let saved = try XCTUnwrap(fixture.service.directory().agents.first { agent in !before.agents.contains { $0.id == agent.id } })
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: saved.defaults).templateID, role.id)
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: saved.defaults).reasoningEffort, "automatic")
        XCTAssertEqual(try fixture.service.roleTemplates().templates.first?.prompt, role.prompt)
        XCTAssertNotNil(element(host, "mac-expert-edit-" + saved.id))
    }

    func testNativeExpertEditorSavesAndShowsRealRoleDetailsInBothAppearances() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let fixture = try nativeFixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let workspace = MacAgentRoomWorkspace(service: fixture.service)
        workspace.agentModelOptions = { try fixture.repository.agentModelOptions() }
        workspace.agentReasoningEfforts = { try fixture.repository.agentReasoningEfforts(modelSelectionID: $0) }
        let host = NSHostingView(rootView: MacExpertDirectoryManagementView(workspace: workspace).background(MacDesignColor.canvas)
            .environment(\.accessibilityEnabled, true).environment(\.colorScheme, .light).preferredColorScheme(.light))
        let window = makeWindow(host, width: 980, height: 780)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await wait { self.element(host, "mac-expert-edit-" + fixture.expert.id) != nil }
        _ = element(host, "mac-expert-edit-" + fixture.expert.id)?.accessibilityPerformPress?()
        try await wait { self.element(host, "mac-expert-role-prompt") != nil }
        XCTAssertNotNil(element(host, "mac-expert-reasoning-effort"))
        XCTAssertNotNil(element(host, "mac-expert-role"))
        XCTAssertNotNil(element(host, "mac-expert-change-template"))
        XCTAssertNil(element(host, "mac-expert-template"), "An applied role should not repeat its template selector")
        let name = try XCTUnwrap(elements(host).compactMap { $0 as? NSTextField }.first { $0.stringValue == fixture.expert.name })
        name.selectText(nil)
        let textEditor = try XCTUnwrap(name.currentEditor() as? NSTextView)
        textEditor.insertText("Native edited expert", replacementRange: NSRange(location: 0, length: textEditor.string.utf16.count))
        window.makeFirstResponder(host)
        try await Task.sleep(for: .milliseconds(100))
        _ = element(host, "mac-expert-save")?.accessibilityPerformPress?()
        try await wait { (try? fixture.service.directory().agents.first?.name) == "Native edited expert" }
        let saved = try XCTUnwrap(fixture.service.directory().agents.first)
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: saved.defaults).reasoningEffort, "high")
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: saved.defaults).templateID, fixture.role.id)
        _ = element(host, "mac-expert-role-prompt")?.accessibilityPerformPress?()
        _ = element(host, "mac-expert-role-tools")?.accessibilityPerformPress?()
        for scheme: ColorScheme in [.light, .dark] {
            host.rootView = MacExpertDirectoryManagementView(workspace: workspace).background(MacDesignColor.canvas)
                .environment(\.accessibilityEnabled, true).environment(\.colorScheme, scheme).preferredColorScheme(scheme)
            window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            host.appearance = window.appearance
            try await Task.sleep(for: .milliseconds(150))
            attach(host, name: "expert-role-editor-\(scheme == .dark ? "dark" : "light")")
        }
        _ = element(host, "mac-expert-change-template")?.accessibilityPerformPress?()
        try await wait { self.element(host, "mac-expert-template") != nil }
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: fixture.service.directory().agents[0].defaults).templateID, fixture.role.id)
    }

    func testNativeAtSelectionJoinsDirectoryExpertByIDWithoutModelRequest() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let fixture = try nativeFixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let workspace = MacAgentRoomWorkspace(service: fixture.service)
        let state = ExpertMentionUITestSelection()
        let model = MacWorkspaceModel(canonicalRepository: fixture.repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(), modelApplication: fixture.port)
        let owner = try XCTUnwrap(model.selectedConversationID)
        model.presentationDidAttach()
        await model.loadSelectedConversationIfNeeded()
        await workspace.restoreConversation(owner)
        let host = NSHostingView(rootView: ExpertMentionUITestRoot(model: model, workspace: workspace, selection: state)
            .environment(\.accessibilityEnabled, true).environment(\.colorScheme, .light).preferredColorScheme(.light))
        let window = makeWindow(host, width: 980, height: 760)
        defer { model.presentationDidDetach(); window.orderOut(nil); window.contentView = nil; window.close() }
        try await Task.sleep(for: .milliseconds(200))
        let input = try XCTUnwrap(views(host).compactMap { $0 as? MacAttachmentPasteTextView }.first)
        XCTAssertTrue(window.makeFirstResponder(input))
        input.insertText("Review @Editor", replacementRange: NSRange(location: NSNotFound, length: 0))
        try await wait { self.element(host, "chat.mention." + fixture.expert.id) != nil }
        _ = element(host, "chat.mention." + fixture.expert.id)?.accessibilityPerformPress?()
        try await wait { state.ids.contains(fixture.expert.id) && model.draft == "Review " }
        let association = try XCTUnwrap(fixture.service.forConversation(owner))
        XCTAssertEqual(association.room.room.participants.first?.agentID, fixture.expert.id)
        XCTAssertTrue(try fixture.service.submissions(roomID: association.roomID).isEmpty, "Mention selection must not call a model")
        for scheme: ColorScheme in [.light, .dark] {
            host.rootView = ExpertMentionUITestRoot(model: model, workspace: workspace, selection: state)
                .environment(\.accessibilityEnabled, true).environment(\.colorScheme, scheme).preferredColorScheme(scheme)
            window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            host.appearance = window.appearance
            try await Task.sleep(for: .milliseconds(150))
            attach(host, name: "expert-at-selected-\(scheme == .dark ? "dark" : "light")")
        }
    }

    func testJoinedSnapshotPinsMainSelectionAndDirectFanoutRequiresMainTurn() async throws {
        let fixture = try nativeFixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let conversation = MacConversation(title: "Snapshot continuity", preview: "", updatedAt: .now, isPinned: false)
        _ = try fixture.repository.ensureConversation(conversation)
        let workspace = MacAgentRoomWorkspace(service: fixture.service)
        await workspace.restoreConversation(conversation.id)
        try await workspace.prepareExpertMention(agentID: fixture.expert.id, expectedRevision: fixture.expert.revision,
            conversationID: conversation.id, isPersisted: true)
        let joined = try XCTUnwrap(fixture.service.forConversation(conversation.id))
        let original = try XCTUnwrap(joined.room.room.participants.first)
        _ = try fixture.service.updateAgent(fixture.expert, name: fixture.expert.name, description: "A newer directory definition",
            availability: "active", directory: fixture.service.directory(),
            defaults: .object(["modelSelectionID": .string("configured:native-fixture"), "expertReasoningEffort": .string("low")]))
        let messageID = UUID().uuidString.lowercased()
        let admitted = await workspace.sendExperts(conversationID: conversation.id, text: "Continue the selected snapshot",
            agentIDs: [fixture.expert.id], messageID: messageID)
        XCTAssertNil(admitted)
        XCTAssertTrue(workspace.admissionError?.contains("requires_main_agent_turn") == true)
        XCTAssertNil(try fixture.service.expertMessageSource(conversationID: conversation.id, messageID: messageID))
        let selected = try await workspace.mainExpertSelection(conversationID: conversation.id, agentIDs: [fixture.expert.id])
        XCTAssertEqual(selected.first?.agentRevision, fixture.expert.revision)
        XCTAssertEqual(selected.first?.contextConversationID, original.contextConversationID)
        let current = try XCTUnwrap(fixture.service.forConversation(conversation.id))
        XCTAssertEqual(current.room.room.participants.first?.id, original.id)
        XCTAssertEqual(current.room.room.participants.first?.agentRevision, original.agentRevision)
        // No execution owner was installed: only Core admission, never inference.
        XCTAssertTrue((workspace.submissions[current.roomID] ?? []).allSatisfy { $0.state != "executing" })
    }

    private func nativeFixture() throws -> (root: URL, port: MacApplicationPort, repository: MacCanonicalConversationRepository,
        service: MacAgentRoomService, expert: MacPersistentAgent, role: AppleExpertRoleTemplate) {
        let root = temporaryRoot()
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("native.sqlite"))
        let source: [String: Any] = ["schemaVersion": 1, "endpointID": "fixture", "endpointName": "Fixture",
            "baseURL": "https://fixture.invalid/v1", "endpointOrder": 0, "endpointEnabled": true,
            "modelEnabled": true, "modelOrder": 0, "priority": 100, "capabilities": ["chat"],
            "wireProtocol": "responses", "credentialAccount": "fixture-account",
            "executionCapabilities": ["toolCalls": true, "imageInput": false, "reasoning": true,
                "samplingParameters": false, "samplingRequiresNoReasoning": false, "chatOutputTokenField": "auto"],
            "reasoning": ["control": "effort", "efforts": ["minimal", "low", "medium", "high", "max"]]]
        try port.upsertModelProfile(.init(id: "native-fixture", displayName: "Declared five levels", provider: "openai-compatible", modelName: "fixture",
            runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: "{}", selected: false, enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
        let service = MacAgentRoomService(runtime: port)
        let role = try XCTUnwrap(service.roleTemplates().templates.first)
        let directory = try service.createAgent(name: "Editor expert", description: "Native control test",
            defaults: .object(["modelSelectionID": .string("configured:native-fixture"),
                               "selectedPromptTemplateID": .string(role.id), "expertReasoningEffort": .string("high")]),
            directory: service.directory())
        return (root, port, MacCanonicalConversationRepository(store: port), service, try XCTUnwrap(directory.agents.first), role)
    }

    private func makeWindow<Content: View>(_ host: NSHostingView<Content>, width: CGFloat, height: CGFloat) -> NSWindow {
        host.frame = NSRect(x: 0, y: 0, width: width, height: height)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = host; window.makeKeyAndOrderFront(nil)
        return window
    }
    private func attach(_ host: NSView, name: String) {
        host.layoutSubtreeIfNeeded()
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { return XCTFail("Native bitmap unavailable") }
        (host.window?.effectiveAppearance ?? NSAppearance.current).performAsCurrentDrawingAppearance {
            host.cacheDisplay(in: host.bounds, to: bitmap)
        }
        guard let data = bitmap.representation(using: .png, properties: [:]) else { return XCTFail("PNG unavailable") }
        let image = XCTAttachment(data: data, uniformTypeIdentifier: "public.png")
        image.name = name; image.lifetime = .keepAlways; add(image)
    }
    private func wait(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(8)
        while !condition(), ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(30)) }
        XCTAssertTrue(condition())
    }
    private func views(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap(views) }
    private func elements(_ root: AnyObject) -> [AnyObject] {
        var queue = [root], seen = Set<ObjectIdentifier>(), index = 0, result: [AnyObject] = []
        while index < queue.count, index < 6000 {
            let object = queue[index]; index += 1
            guard seen.insert(ObjectIdentifier(object)).inserted else { continue }
            result.append(object)
            let children = object.accessibilityChildren?() ?? []
            queue += (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject }
            if let view = object as? NSView { queue += view.subviews }
        }
        return result
    }
    private func element(_ root: AnyObject, _ id: String) -> AnyObject? {
        elements(root).first { ($0.accessibilityIdentifier?() ?? nil) == id }
    }

    private func temporaryRoot() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("expert-role-\(UUID())")
    }
}

@MainActor private final class ExpertMentionUITestSelection: ObservableObject {
    @Published var ids: Set<String> = []
}
private struct ExpertMentionUITestRoot: View {
    let model: MacWorkspaceModel
    @ObservedObject var workspace: MacAgentRoomWorkspace
    @ObservedObject var selection: ExpertMentionUITestSelection
    @FocusState private var focused: Bool
    var body: some View {
        MacConversationCanvas(model: model, composerFocused: $focused,
            expertSelection: $selection.ids, expertWorkspace: workspace)
            .background(MacDesignColor.canvas).tint(MacDesignColor.brand).onAppear { focused = true }
    }
}
