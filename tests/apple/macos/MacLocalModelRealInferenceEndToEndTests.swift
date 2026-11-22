// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Opt-in end-to-end validation of a downloaded MLX model through Nola's loopback API.

import AppKit
import SwiftUI
import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacLocalModelRealInferenceEndToEndTests: XCTestCase {
    func testDownloadedGGUFServesChatThroughNativeRoute() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard let fixture = environment["NOLA_TEST_HUB_SNAPSHOT_ROOT"],
              environment["NOLA_LLAMA_SERVER"] != nil else {
            throw XCTSkip("Set NOLA_TEST_HUB_SNAPSHOT_ROOT and NOLA_LLAMA_SERVER to exercise downloaded GGUF weights.")
        }
        let seed = try XCTUnwrap(MacLocalModelProfile.installed(in: URL(fileURLWithPath: fixture)).first { $0.engine == .llamaCpp })
        let repository = try XCTUnwrap(seed.repository)
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-gguf-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let library = root.appendingPathComponent("models")
        let flow = MacLocalModelAcquisition(client: .init(root: library))
        let suite = "nola.model.hub.tests.\(UUID().uuidString)"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: suite))
        preferences.set("llama.cpp", forKey: "nola.local-model.engine")
        defer { preferences.removePersistentDomain(forName: suite) }
        let sheet = MacModelAcquisitionSheet(acquisition: flow, source: .constant("huggingface"))
            .defaultAppStorage(preferences).environment(\.accessibilityEnabled, true)
            .background(Color(nsColor: .windowBackgroundColor))
        let host = NSHostingView(rootView: sheet)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 560),
            styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.close(); flow.reset() }
        let ax = try MacHostedAccessibilitySession()
        defer { ax.restore() }
        func wait(_ condition: () -> Bool) async throws {
            let deadline = ContinuousClock.now + .seconds(180)
            while !condition() {
                if case let .failed(message) = flow.phase { throw NSError(domain: "ModelAcquisition", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
                if ContinuousClock.now >= deadline { throw CocoaError(.executableNotLoadable) }
                try await Task.sleep(for: .milliseconds(75))
            }
        }
        func press(_ identifier: String) async throws {
            try await MacVisualPixelAssertions.settleLayout(window: window, hostingView: host)
            let button = try XCTUnwrap(ax.element(identifier, in: host), identifier)
            XCTAssertTrue(button.accessibilityPerformPress?() == true, identifier)
        }
        try await wait { flow.hardware != nil || flow.hardwareError != nil }
        XCTAssertNotNil(flow.hardware?.memoryBytes, flow.hardwareError ?? "")
        try await wait { flow.phase == .choosing }
        XCTAssertNotNil(flow.recommendationSource)
        XCTAssertFalse(FileManager.default.fileExists(atPath: library.appendingPathComponent("catalog.json").path))
        try await MacVisualPixelAssertions.settleLayout(window: window, hostingView: host)
        let automatic = try MacVisualPixelAssertions.captureCGImage(of: host.superview ?? host)
        let autoData = try XCTUnwrap(NSBitmapImageRep(cgImage: automatic).representation(using: .png, properties: [:]))
        let autoCapture = XCTAttachment(data: autoData, uniformTypeIdentifier: "public.png")
        autoCapture.name = "automatic-hardware-based-recommendations"
        autoCapture.lifetime = .keepAlways
        add(autoCapture)
        func nameField(_ view: NSView) -> NSTextField? {
            if let field = view as? NSTextField, field.isEditable { return field }
            return view.subviews.lazy.compactMap { nameField($0) }.first
        }
        let name = try XCTUnwrap(nameField(host))
        XCTAssertTrue(window.makeFirstResponder(name))
        let editor = try XCTUnwrap(name.currentEditor() as? NSTextView)
        editor.insertText(repository, replacementRange: editor.selectedRange())
        try await wait { flow.query == repository && flow.phase == .idle }
        try await press("local-acquire-submit")
        try await wait { flow.phase == .choosing }
        let index = try XCTUnwrap(flow.candidates.firstIndex { $0.repository == repository }, flow.reason ?? "")
        let offer = flow.candidates[index]
        XCTAssertGreaterThan(offer.contextTokens, 0)
        XCTAssertGreaterThan(offer.estimatedMemoryBytes, 0)
        XCTAssertFalse(FileManager.default.fileExists(atPath: library.appendingPathComponent("catalog.json").path))
        let previewFiles = FileManager.default.enumerator(atPath: library.path)?.allObjects.compactMap { $0 as? String } ?? []
        XCTAssertFalse(previewFiles.contains { $0.hasSuffix(".gguf") || $0.hasSuffix(".part") })
        try await MacVisualPixelAssertions.settleLayout(window: window, hostingView: host)
        let cgImage = try MacVisualPixelAssertions.captureCGImage(of: host.superview ?? host)
        let screenshot = try XCTUnwrap(NSBitmapImageRep(cgImage: cgImage).representation(using: .png, properties: [:]))
        let preview = XCTAttachment(data: screenshot, uniformTypeIdentifier: "public.png")
        preview.name = "hardware-and-recommended-configuration-before-confirmation"
        preview.lifetime = .keepAlways
        add(preview)
        try await press("local-acquire-confirm-\(index)")
        try await wait { if case .installed = flow.phase { return true }; return false }
        let profile = try XCTUnwrap(MacLocalModelProfile.installed(in: library).first)
        XCTAssertEqual(profile.recommendedContextTokens, offer.contextTokens)
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("inference.sqlite"))
        let service = MacMLXOpenAIService.shared
        defer { service.stop() }
        let endpoint = try await service.start(profile, applicationPort: port)
        XCTAssertEqual(try endpoint.nativeRoute()["adapter"] as? String, "llama_cpp")
        XCTAssertEqual(try port.modelProfile(profile.id)?.runtime, "llama.cpp")
        XCTAssertEqual(endpoint.contextWindowTokens, offer.contextTokens)
        try service.validate(endpoint, applicationPort: port)
        try await verifySingleSlotAndChat(endpoint)
        service.stop()
        XCTAssertFalse(service.isRunning(profile))
    }

    func testDownloadedModelOpensConversationWithItsOwnProfile() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard let fixture = environment["NOLA_TEST_HUB_SNAPSHOT_ROOT"], environment["NOLA_LLAMA_SERVER"] != nil,
              URL(fileURLWithPath: fixture).standardizedFileURL.path == MacLocalModelProfile.modelsRoot.standardizedFileURL.path else {
            throw XCTSkip("Use an isolated NOLA_MODELS_ROOT matching NOLA_TEST_HUB_SNAPSHOT_ROOT and an installed NOLA_LLAMA_SERVER.")
        }
        let profile = try XCTUnwrap(MacLocalModelProfile.installed.first { $0.engine == .llamaCpp })
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-model-chat-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("chat.sqlite"))
        let preference = MacDefaultChatModelPresentation(store: .init(application: application))
        let originalDefault = try preference.store.rawSnapshot()
        let editing = MacLocalModelEditingState()
        editing.inspectedModelID = profile.id
        editing.showsCompactDetail = true
        let flow = MacLocalModelAcquisition()
        var opened: String?
        let content = MacLocalModelManagerView(preference: preference, editingState: editing, acquisition: flow,
            openConversation: { opened = $0 }).environment(\.accessibilityEnabled, true)
        let host = NSHostingView(rootView: content)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1080, height: 760),
            styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.close(); MacMLXOpenAIService.shared.stop() }
        let ax = try MacHostedAccessibilitySession()
        defer { ax.restore() }
        let deadline = ContinuousClock.now + .seconds(30)
        while flow.engines[.llamaCpp]?.ready != true && ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(70))
        }
        XCTAssertTrue(flow.engines[.llamaCpp]?.ready == true)
        try await MacVisualPixelAssertions.settleLayout(window: window, hostingView: host)
        let button = try XCTUnwrap(ax.element("local-model-chat-" + profile.id, in: host))
        XCTAssertEqual(button.isAccessibilityEnabled?(), true)
        let capture = try MacVisualPixelAssertions.captureCGImage(of: host.superview ?? host)
        let png = try XCTUnwrap(NSBitmapImageRep(cgImage: capture).representation(using: .png, properties: [:]))
        let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
        attachment.name = "installed-model-ready-to-chat"; attachment.lifetime = .keepAlways; add(attachment)
        XCTAssertTrue(button.accessibilityPerformPress?() == true)
        let chatDeadline = ContinuousClock.now + .seconds(45)
        while opened == nil && ContinuousClock.now < chatDeadline { try await Task.sleep(for: .milliseconds(60)) }
        let saved = try XCTUnwrap(MacCanonicalConversationRepository(store: application).load(conversationID: try XCTUnwrap(opened)))
        XCTAssertEqual(saved.conversation.modelSelectionID, MacChatModelOption(source: .local, modelID: profile.id, displayName: profile.displayName).id)
        XCTAssertEqual(saved.conversation.executionHostRawValue, "local")
        XCTAssertEqual(try preference.store.rawSnapshot().valueJSON, originalDefault.valueJSON)
        let endpoint = try MacMLXOpenAIService.shared.endpoint(for: profile)
        try await verifySingleSlotAndChat(endpoint)
    }

    func testConfirmed128KModelServesOneFullContextSlot() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard let library = environment["NOLA_TEST_HUB_CONFIRMED_128K_ROOT"], environment["NOLA_LLAMA_SERVER"] != nil else {
            throw XCTSkip("Set NOLA_TEST_HUB_CONFIRMED_128K_ROOT and NOLA_LLAMA_SERVER to use a confirmed long-context GGUF.")
        }
        let profile = try XCTUnwrap(MacLocalModelProfile.installed(in: URL(fileURLWithPath: library)).first {
            $0.engine == .llamaCpp && ($0.recommendedContextTokens ?? 0) >= 131_072
        })
        let repository = try XCTUnwrap(profile.repository)
        let flow = MacLocalModelAcquisition(client: .init(root: URL(fileURLWithPath: library)))
        flow.query = repository
        let suite = "nola.128k.preview.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defaults.set("llama.cpp", forKey: "nola.local-model.engine")
        defer { defaults.removePersistentDomain(forName: suite) }
        let host = NSHostingView(rootView: MacModelAcquisitionSheet(acquisition: flow, source: .constant("huggingface"))
            .defaultAppStorage(defaults).background(Color(nsColor: .windowBackgroundColor)))
        let previewWindow = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 520),
            styleMask: [.titled, .closable], backing: .buffered, defer: false)
        previewWindow.isReleasedWhenClosed = false
        previewWindow.contentView = host
        previewWindow.orderFrontRegardless()
        defer { previewWindow.close(); flow.reset() }
        let deadline = ContinuousClock.now + .seconds(90)
        while flow.phase != .choosing && ContinuousClock.now < deadline {
            if case .failed(let message) = flow.phase {
                throw NSError(domain: "128K recommendation", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        let offered = try XCTUnwrap(flow.candidates.first { $0.repository == repository }, flow.reason ?? "")
        XCTAssertTrue(offered.estimates128K)
        try await MacVisualPixelAssertions.settleLayout(window: previewWindow, hostingView: host)
        let capture = try MacVisualPixelAssertions.captureCGImage(of: host.superview ?? host)
        let screenshot = try XCTUnwrap(NSBitmapImageRep(cgImage: capture).representation(using: .png, properties: [:]))
        let attachment = XCTAttachment(data: screenshot, uniformTypeIdentifier: "public.png")
        attachment.name = "128K model recommendation before confirmation"
        attachment.lifetime = .keepAlways
        add(attachment)
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-128k-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("inference.sqlite"))
        let service = MacMLXOpenAIService.shared
        defer { service.stop() }
        let endpoint = try await service.start(profile, applicationPort: application)
        XCTAssertEqual(endpoint.contextWindowTokens, profile.recommendedContextTokens)
        try await verifySingleSlotAndChat(endpoint)
    }

    private func verifySingleSlotAndChat(_ endpoint: MacMLXOpenAIService.Endpoint) async throws {
        let baseURL = try XCTUnwrap(URL(string: endpoint.baseURL))
        var slotRequest = URLRequest(url: baseURL.deletingLastPathComponent().appendingPathComponent("slots"))
        slotRequest.setValue("Bearer " + endpoint.apiKey, forHTTPHeaderField: "Authorization")
        let (slotData, slotResponse) = try await URLSession.shared.data(for: slotRequest)
        XCTAssertEqual((slotResponse as? HTTPURLResponse)?.statusCode, 200)
        let slots = try XCTUnwrap(JSONSerialization.jsonObject(with: slotData) as? [[String: Any]])
        XCTAssertEqual(slots.count, 1)
        let window = try XCTUnwrap(slots.first?["n_ctx"] as? Int)
        XCTAssertEqual(window, endpoint.contextWindowTokens)
        let evidence = XCTAttachment(string: "\(endpoint.modelID) · \(slots.count) slot · \(window) tokens")
        evidence.name = "llama.cpp reported per-slot context"
        evidence.lifetime = .keepAlways
        add(evidence)
        var request = URLRequest(url: try XCTUnwrap(URL(string: endpoint.baseURL + "/chat/completions")))
        request.httpMethod = "POST"
        request.setValue("Bearer " + endpoint.apiKey, forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: [
            "model": endpoint.modelID, "messages": [["role": "user", "content": "Name one color."]],
            "max_tokens": 24, "temperature": 0, "stream": false,
        ])
        let (body, response) = try await URLSession.shared.data(for: request)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200, String(decoding: body, as: UTF8.self))
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: body) as? [String: Any])
        let choices = try XCTUnwrap(json["choices"] as? [[String: Any]])
        let message = try XCTUnwrap(choices.first?["message"] as? [String: Any])
        let content = try XCTUnwrap(message["content"] as? String)
        XCTAssertFalse(content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        let attachment = XCTAttachment(string: content)
        attachment.name = "GGUF model reply"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func testQwen35Point8BStartsServesAChatAndStops() async throws {
        let profile = MacLocalModelProfile.qwen35_08B
        let modelDirectory = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("models", isDirectory: true)
            .appendingPathComponent(profile.id, isDirectory: true)
        let weights = modelDirectory.appendingPathComponent("model.safetensors")
        guard FileManager.default.fileExists(atPath: weights.path) else {
            throw XCTSkip("Download Qwen3.5 0.8B before running the real local-model test.")
        }
        let weightAttributes = try FileManager.default.attributesOfItem(atPath: weights.path)
        let weightSize = try XCTUnwrap(weightAttributes[.size] as? NSNumber).int64Value
        XCTAssertEqual(weightSize, 625_229_487, "The real inference test requires the complete 0.8B weight file.")

        let service = MacMLXOpenAIService.shared
        service.stop()
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-mlx-native-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("native.sqlite"))
        let phaseCollector = Task { @MainActor in
            var phases: [MacLocalModelRuntimePhase] = []
            for await status in service.statusUpdates() {
                if phases.last != status.phase {
                    phases.append(status.phase)
                }
                if status.phase == .ready || status.phase == .failed {
                    return phases
                }
            }
            return phases
        }
        let startTask = Task { try await service.start(profile, applicationPort: port) }
        let concurrentStartTask = Task { try await service.start(profile, applicationPort: port) }
        let endpoint = try await startTask.value
        let concurrentEndpoint = try await concurrentStartTask.value
        let observedPhases = await phaseCollector.value
        defer { service.stop() }

        XCTAssertTrue(service.isRunning(profile))
        XCTAssertTrue(observedPhases.contains(.loadingWeights), "Observed phases: \(observedPhases)")
        XCTAssertTrue(observedPhases.contains(.ready), "Observed phases: \(observedPhases)")
        XCTAssertEqual(endpoint.modelID, profile.id)
        XCTAssertEqual(concurrentEndpoint.baseURL, endpoint.baseURL)
        XCTAssertEqual(concurrentEndpoint.apiKey, endpoint.apiKey)
        XCTAssertGreaterThanOrEqual(endpoint.contextWindowTokens, 65_536)

        var modelsRequest = URLRequest(url: try XCTUnwrap(URL(string: endpoint.baseURL + "/models")))
        modelsRequest.setValue("Bearer \(endpoint.apiKey)", forHTTPHeaderField: "Authorization")
        let (modelsData, modelsResponse) = try await URLSession.shared.data(for: modelsRequest)
        XCTAssertEqual((modelsResponse as? HTTPURLResponse)?.statusCode, 200)
        let modelsJSON = try XCTUnwrap(JSONSerialization.jsonObject(with: modelsData) as? [String: Any])
        let listedModels = try XCTUnwrap(modelsJSON["data"] as? [[String: Any]])
        XCTAssertTrue(listedModels.contains { $0["id"] as? String == profile.id })

        var chatRequest = URLRequest(url: try XCTUnwrap(URL(string: endpoint.baseURL + "/chat/completions")))
        chatRequest.httpMethod = "POST"
        chatRequest.setValue("Bearer \(endpoint.apiKey)", forHTTPHeaderField: "Authorization")
        chatRequest.setValue("application/json", forHTTPHeaderField: "Content-Type")
        chatRequest.httpBody = try JSONSerialization.data(withJSONObject: [
            "model": profile.id,
            "stream": false,
            "messages": [
                ["role": "system", "content": "Answer briefly and directly."],
                ["role": "user", "content": "Reply with exactly the word OK."],
            ],
        ])

        let (chatData, chatResponse) = try await URLSession.shared.data(for: chatRequest)
        XCTAssertEqual(
            (chatResponse as? HTTPURLResponse)?.statusCode,
            200,
            String(decoding: chatData, as: UTF8.self)
        )
        let chatJSON = try XCTUnwrap(JSONSerialization.jsonObject(with: chatData) as? [String: Any])
        guard
            let choices = chatJSON["choices"] as? [[String: Any]],
            let firstChoice = choices.first,
            let message = firstChoice["message"] as? [String: Any],
            let content = message["content"] as? String
        else {
            XCTFail("Unexpected chat response: \(String(decoding: chatData, as: UTF8.self))")
            return
        }
        XCTAssertFalse(content.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines).isEmpty)

        // A fresh execution channel is allowed to rotate its token and port,
        // while the logical canonical profile/model identity stays exact.
        let originalRoute = try endpoint.nativeRoute()
        let originalProfile = try XCTUnwrap(port.modelProfile(profile.id))
        service.stop()
        let replacement = try await service.start(profile, expectedRevision: originalProfile.revision,
            applicationPort: port)
        XCTAssertNotEqual(replacement.apiKey, endpoint.apiKey)
        XCTAssertTrue(NSDictionary(dictionary: originalRoute).isEqual(to: try replacement.nativeRoute()))
        XCTAssertEqual(try port.modelProfile(profile.id), originalProfile, "A channel restart cannot rewrite model authority.")
        XCTAssertThrowsError(try service.validate(endpoint, applicationPort: port))
        let proof: [String: Any] = ["native_model_route": originalRoute,
            "endpoint": replacement.baseURL, "model": replacement.modelID,
            "credential_sha256": MacRunModelCredential.digest(replacement.apiKey)]
        try MacAgentHostRuntime.validateNativeModelChannel(proof, endpoint: replacement, applicationPort: port)
        var foreign = proof
        foreign["model"] = "foreign-model"
        do {
            try MacAgentHostRuntime.validateNativeModelChannel(foreign, endpoint: replacement, applicationPort: port)
            XCTFail("A proxy channel must never authorize another model.")
        } catch {}
        let settings = MacLocalModelConfigurationStore(applicationPort: port)
        var changed = settings.parameters(for: profile)
        changed.temperature = 0.123
        _ = try settings.saveParameters(changed, for: profile, expectedRevision: UInt64(originalProfile.revision))
        XCTAssertThrowsError(try service.validate(replacement, applicationPort: port))
        do {
            _ = try await service.start(profile, expectedRevision: originalProfile.revision, applicationPort: port)
            XCTFail("Changed profile authority must refuse the original frozen Run.")
        } catch {}
    }
}
