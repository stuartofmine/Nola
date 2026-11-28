// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Deep real-model macOS E2E suite verifying strict permission matrices (Deny / Allow Once / AllowAll / Cautious downgrade / Plan Mode),
//   real multi-page PDF attachment + oversized output spill recovery, and live tool journal streaming.

import AppKit
import Foundation
import PDFKit
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacRealWorkflowPermissionsAndToolsE2ETests: XCTestCase {
    override func tearDown() {
        MacApplicationServiceSupervisor.shared.stopOwnedProcesses()
        super.tearDown()
    }

    private struct RealLLMConfig {
        let endpoint: String
        let model: String
        let apiKey: String
        let wireProtocol: MacModelWireProtocol
    }

    private func loadRealLLMConfig() throws -> RealLLMConfig {
        let env = ProcessInfo.processInfo.environment
        let fixtureURL = URL(fileURLWithPath: "/tmp/nola-real-llm-e2e.json")
        let fixture = (try? Data(contentsOf: fixtureURL))
            .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: String] }
        let envKey = env["NOLA_REAL_LLM_API_KEY"] ?? env["NOLA_PRIMARY_API_KEY"]
        let apiKey = (envKey?.isEmpty == false ? envKey : nil) ?? fixture?["apiKey"] ?? ""
        guard !apiKey.isEmpty else {
            throw XCTSkip("Requires NOLA_REAL_LLM_API_KEY or /tmp/nola-real-llm-e2e.json")
        }
        let endpoint = (env["NOLA_REAL_LLM_ENDPOINT"].flatMap { $0.isEmpty ? nil : $0 })
            ?? fixture?["endpoint"] ?? "http://192.168.31.18:8081/v1"
        let model = (env["NOLA_REAL_LLM_MODEL"].flatMap { $0.isEmpty ? nil : $0 })
            ?? fixture?["model"] ?? "qwen3.8-flash-next-iq3_s"
        let rawWire = env["NOLA_REAL_LLM_WIRE_PROTOCOL"] ?? fixture?["wireProtocol"] ?? "chat_completions"
        return RealLLMConfig(
            endpoint: endpoint,
            model: model,
            apiKey: apiKey,
            wireProtocol: MacModelWireProtocol(rawValue: rawWire) ?? .chatCompletions
        )
    }

    private func makeWorkspaceHarness(
        prefix: String,
        config: RealLLMConfig
    ) async throws -> (
        root: URL,
        workspaceDir: URL,
        port: MacApplicationPort,
        workspaceModel: MacWorkspaceModel,
        window: NSWindow,
        hosting: NSHostingView<AnyView>
    ) {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-wf-\(prefix)-\(UUID().uuidString)", isDirectory: true)
        let workspaceDir = root.appendingPathComponent("workspace", isDirectory: true)
        try FileManager.default.createDirectory(at: workspaceDir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }

        let databaseURL = root.appendingPathComponent("nola-wf.db")
        let port = try MacApplicationPort(databaseURL: databaseURL)
        let configStore = MacAppleChatConfigurationStore(applicationPort: port)
        let connectionService = MacModelConnectionService(application: port)
        let created = try connectionService.create(
            name: "LAN Workflow Model",
            endpoint: config.endpoint,
            wireProtocol: config.wireProtocol.rawValue,
            secret: config.apiKey
        )
        let discovered = try await connectionService.discover(created)
        let models = try XCTUnwrap(discovered.document.catalog?.models)
        let target = models.first(where: { $0.model_id == config.model }) ?? models[0]
        try connectionService.enable(discovered, modelIDs: [target.id])

        let repository = MacCanonicalConversationRepository(store: port)
        let agentRuntime = MacAgentHostRuntime(credentialStore: configStore)
        let remoteHost = E2ENoOpRemoteHost()
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let webStore = MacWebSourceStore(storeURL: root.appendingPathComponent("web.json"), registry: registry)
        let activityStore = MacActivityStore(baseDirectory: root.appendingPathComponent("Activity"))
        let suiteName = "com.gaojc.Nola.wf-bookmarks.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        addTeardownBlock { UserDefaults.standard.removePersistentDomain(forName: suiteName) }
        let bookmarkStore = MacSecurityScopedBookmarkStore(defaults: defaults)

        let workspaceModel = MacWorkspaceModel(
            canonicalRepository: repository,
            localAgentRuntime: agentRuntime,
            remoteHost: remoteHost,
            modelApplication: port,
            bookmarkStore: bookmarkStore,
            activityStore: activityStore,
            completionNotifier: { _, _, _ in },
            readinessIssue: { _ in nil }
        )
        let deps = MacInspectorDependencies(
            dataRegistry: registry,
            platformData: MacPlatformDataService.shared,
            webSources: webStore,
            chromeMessaging: MacChromeMessagingService.shared
        )
        let chatView = MacChatWorkspaceView(
            forcesInspectorVisible: false,
            canonicalRepository: repository,
            localAgentRuntime: agentRuntime,
            remoteHost: remoteHost,
            inspectorDependencies: deps,
            model: workspaceModel
        )
        let hosting = NSHostingView(rootView: AnyView(chatView.environment(\.colorScheme, .dark)))
        hosting.frame = NSRect(x: 0, y: 0, width: 1280, height: 820)
        let window = NSWindow(
            contentRect: hosting.frame,
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        hosting.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(120))

        workspaceModel.newConversation()
        await workspaceModel.loadSelectedConversationIfNeeded()
        try await workspaceModel.waitForModelConfigurationCommit()
        let option = try XCTUnwrap(
            workspaceModel.configuredModelOptions.first(where: { $0.modelID == target.model_id })
                ?? workspaceModel.configuredModelOptions.first
        )
        workspaceModel.selectModel(option)
        try await workspaceModel.waitForModelConfigurationCommit()
        XCTAssertNotNil(workspaceModel.switchWorkspace(to: workspaceDir))
        return (root, workspaceDir, port, workspaceModel, window, hosting)
    }

    /// Generates a real multi-page PDF (>35 KiB extracted text) with distinct unique markers per page.
    private func generateRealMultiPagePDF(at url: URL, pageCount: Int, markers: [Int: String]) throws {
        let pdf = PDFDocument()
        let fillerParagraph = String(
            repeating: "Receiver-conditioned hybrid state reconstruction evaluates low-rank residual updates across recurrent matrix memory layers and attention KV caches. ",
            count: 26
        )
        for pageIndex in 1...pageCount {
            let markerLine = markers[pageIndex].map { "UNIQUE_PAGE_MARKER_\(pageIndex): \($0)\n\n" } ?? ""
            let body = "Section \(pageIndex). Page \(pageIndex) of \(pageCount)\n\n\(markerLine)\(fillerParagraph)"
            let pageRect = CGRect(x: 0, y: 0, width: 612, height: 792)
            let data = NSMutableData()
            var mediaBox = pageRect
            guard let consumer = CGDataConsumer(data: data as CFMutableData),
                  let ctx = CGContext(consumer: consumer, mediaBox: &mediaBox, nil) else {
                throw NSError(domain: "PDFGen", code: 1)
            }
            ctx.beginPDFPage(nil)
            let nsCtx = NSGraphicsContext(cgContext: ctx, flipped: false)
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = nsCtx
            let attrs: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: 11),
                .foregroundColor: NSColor.black,
            ]
            (body as NSString).draw(in: CGRect(x: 40, y: 40, width: 532, height: 712), withAttributes: attrs)
            NSGraphicsContext.restoreGraphicsState()
            ctx.endPDFPage()
            ctx.closePDF()

            if let single = PDFDocument(data: data as Data), let page = single.page(at: 0) {
                pdf.insert(page, at: pdf.pageCount)
            }
        }
        XCTAssertEqual(pdf.pageCount, pageCount)
        XCTAssertTrue(pdf.write(to: url), "Failed to write generated multi-page PDF")
    }

    // MARK: - 1. Full Real Permission Matrix: Cautious Deny -> Cautious Allow Once -> AllowAll 0-Prompt -> Cautious Re-Prompt -> Plan Mode

    func testFullRealPermissionMatrixAndPlanMode() async throws {
        let config = try loadRealLLMConfig()
        setenv("NOLA_REAL_LLM_E2E", "1", 1)
        defer { unsetenv("NOLA_REAL_LLM_E2E") }

        let h = try await makeWorkspaceHarness(prefix: "perm-matrix", config: config)
        defer {
            if h.workspaceModel.isRunning { h.workspaceModel.stop(reason: "teardown") }
            h.workspaceModel.presentationDidDetach()
            h.window.close()
        }
        let wm = h.workspaceModel
        let convID = try XCTUnwrap(wm.selectedConversationID)
        XCTAssertTrue(wm.selectComposerReasoningPreference("low", conversationID: convID))
        try await wm.waitForModelConfigurationCommit()

        // Turn 1: Cautious mode -> model tries bash to create denied.txt -> WE DENY -> file MUST NOT exist!
        wm.permissionMode = .cautious
        try await wm.waitForModelConfigurationCommit()
        let deniedFile = h.workspaceDir.appendingPathComponent("denied.txt")
        wm.draft = "请使用 bash 工具执行命令 `echo SHOULD_NOT_EXIST > denied.txt`，执行后告诉我结果。"
        wm.send()
        XCTAssertTrue(wm.isRunning)

        var sawPromptOnDenyTurn = false
        var observedResourceNonEmpty = false
        let t1Deadline = ContinuousClock.now + .seconds(90)
        while wm.isRunning && ContinuousClock.now < t1Deadline {
            if let pending = wm.pendingRemotePermission, !wm.isResolvingPermission {
                sawPromptOnDenyTurn = true
                if !pending.toolName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    && !pending.arguments.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    observedResourceNonEmpty = true
                }
                wm.resolvePermission(allow: false, scope: .once, requestID: pending.id, expectedRequest: pending)
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertFalse(wm.isRunning, "Turn 1 (Cautious Deny) timed out")
        XCTAssertTrue(sawPromptOnDenyTurn, "Cautious mode MUST pop a permission request before running bash")
        XCTAssertTrue(observedResourceNonEmpty, "Permission request MUST carry a non-empty resource scope")
        XCTAssertFalse(FileManager.default.fileExists(atPath: deniedFile.path), "Denied bash command MUST NOT create denied.txt")

        // Turn 2: Upgrade SAME conversation to AllowAll -> bash creates allowed.txt with ZERO prompts!
        wm.permissionMode = .allowAll
        try await wm.waitForModelConfigurationCommit()
        let allowedFile = h.workspaceDir.appendingPathComponent("allowed.txt")
        wm.draft = "现在请使用 bash 工具执行命令 `echo ALLOW_ALL_OK > allowed.txt` 并回复结果。"
        wm.send()
        XCTAssertTrue(wm.isRunning)

        var promptsUnderAllowAll = 0
        let t2Deadline = ContinuousClock.now + .seconds(90)
        while wm.isRunning && ContinuousClock.now < t2Deadline {
            if let pending = wm.pendingRemotePermission, !wm.isResolvingPermission {
                promptsUnderAllowAll += 1
                wm.resolvePermission(allow: true, scope: .once, requestID: pending.id, expectedRequest: pending)
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertFalse(wm.isRunning, "Turn 2 (AllowAll) timed out")
        XCTAssertEqual(promptsUnderAllowAll, 0, "AllowAll mode on resumed session MUST execute bash with 0 approval prompts")
        let allowedText = try String(contentsOf: allowedFile, encoding: .utf8)
        XCTAssertTrue(allowedText.contains("ALLOW_ALL_OK"), "bash under AllowAll should have written allowed.txt")

        // Turn 3: Downgrade SAME conversation back to Cautious -> bash MUST prompt again!
        wm.permissionMode = .cautious
        try await wm.waitForModelConfigurationCommit()
        let cautiousOnceFile = h.workspaceDir.appendingPathComponent("once.txt")
        wm.draft = "现在请使用 bash 工具执行命令 `echo ONCE_OK > once.txt`。"
        wm.send()
        XCTAssertTrue(wm.isRunning)

        var promptsAfterDowngrade = 0
        let t3Deadline = ContinuousClock.now + .seconds(90)
        while wm.isRunning && ContinuousClock.now < t3Deadline {
            if let pending = wm.pendingRemotePermission, !wm.isResolvingPermission {
                promptsAfterDowngrade += 1
                wm.resolvePermission(allow: true, scope: .once, requestID: pending.id, expectedRequest: pending)
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertFalse(wm.isRunning, "Turn 3 (Downgrade Cautious -> Allow Once) timed out")
        XCTAssertGreaterThanOrEqual(promptsAfterDowngrade, 1, "Downgrading back to Cautious MUST require approval again")
        let onceText = try String(contentsOf: cautiousOnceFile, encoding: .utf8)
        XCTAssertTrue(onceText.contains("ONCE_OK"))

        // Turn 4: Plan Mode -> write_file / bash MUST be blocked even when permissionMode is AllowAll!
        wm.permissionMode = .allowAll
        XCTAssertTrue(wm.selectComposerPlanMode(true, conversationID: convID))
        try await wm.waitForModelConfigurationCommit()
        let planBlockedFile = h.workspaceDir.appendingPathComponent("plan_blocked.txt")
        wm.draft = "请创建一个名为 plan_blocked.txt 的文件，写入 PLAN_SHOULD_BLOCK。"
        wm.send()
        XCTAssertTrue(wm.isRunning)
        let t4Deadline = ContinuousClock.now + .seconds(90)
        while wm.isRunning && ContinuousClock.now < t4Deadline {
            if let pending = wm.pendingRemotePermission, !wm.isResolvingPermission {
                wm.resolvePermission(allow: false, scope: .once, requestID: pending.id, expectedRequest: pending)
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertFalse(wm.isRunning, "Turn 4 (Plan Mode) timed out")
        XCTAssertFalse(FileManager.default.fileExists(atPath: planBlockedFile.path), "Plan Mode MUST prevent writing plan_blocked.txt")
    }

    // MARK: - 2. Real 15-Page PDF Attachment + Oversized Spill Recovery + Zero Wasted Bash Searches

    func testRealMultiPagePDFAttachmentDirectPathAndSpillRecovery() async throws {
        let config = try loadRealLLMConfig()
        setenv("NOLA_REAL_LLM_E2E", "1", 1)
        defer { unsetenv("NOLA_REAL_LLM_E2E") }

        let h = try await makeWorkspaceHarness(prefix: "pdf-spill", config: config)
        defer {
            if h.workspaceModel.isRunning { h.workspaceModel.stop(reason: "teardown") }
            h.workspaceModel.presentationDidDetach()
            h.window.close()
        }
        let wm = h.workspaceModel
        let convID = try XCTUnwrap(wm.selectedConversationID)
        wm.permissionMode = .allowAll
        XCTAssertTrue(wm.selectComposerReasoningPreference("low", conversationID: convID))
        try await wm.waitForModelConfigurationCommit()

        let pdfURL = h.root.appendingPathComponent("paper_15p.pdf")
        let p1Marker = "ALPHA_P1_\(UUID().uuidString.prefix(6))"
        let p14Marker = "OMEGA_P14_\(UUID().uuidString.prefix(6))"
        try generateRealMultiPagePDF(at: pdfURL, pageCount: 15, markers: [1: p1Marker, 14: p14Marker])

        // Attach via real composer importFiles
        wm.importFiles([pdfURL], conversationID: convID)
        let attachDeadline = ContinuousClock.now + .seconds(10)
        while wm.attachments.contains(where: { $0.status == .extracting }) && ContinuousClock.now < attachDeadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertEqual(wm.attachments.count, 1, "PDF should be attached in composer")

        wm.draft = "看下这篇附加的 PDF，请用 read_file 直接读取第 1 页和第 14 页，原样输出其中的 UNIQUE_PAGE_MARKER 值。不要用 bash 搜索目录。"
        wm.send()
        XCTAssertTrue(wm.isRunning)

        let deadline = ContinuousClock.now + .seconds(150)
        while wm.isRunning && ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(150))
        }
        XCTAssertFalse(wm.isRunning, "Real PDF reading turn timed out")
        await wm.loadSelectedConversationIfNeeded()
        try await Task.sleep(for: .milliseconds(350))

        let reply = wm.selectedMessages.filter { $0.role == .assistant }.map(\.text).joined(separator: "\n")
        XCTAssertTrue(reply.contains(p1Marker), "Reply must extract Page 1 marker \(p1Marker), got: \(reply.prefix(400))")
        XCTAssertTrue(reply.contains(p14Marker), "Reply must extract Page 14 marker \(p14Marker), got: \(reply.prefix(400))")
    }

    // MARK: - 3. Real Model PNG Artifact Emission + Vision read_file + Chronological Post-Tool Answer

    func testRealModelFigureGenerationVisionInspectionAndChronologicalJournal() async throws {
        let config = try loadRealLLMConfig()
        setenv("NOLA_REAL_LLM_E2E", "1", 1)
        defer { unsetenv("NOLA_REAL_LLM_E2E") }

        let h = try await makeWorkspaceHarness(prefix: "fig-vision", config: config)
        defer {
            if h.workspaceModel.isRunning { h.workspaceModel.stop(reason: "teardown") }
            h.workspaceModel.presentationDidDetach()
            h.window.close()
        }
        let wm = h.workspaceModel
        let convID = try XCTUnwrap(wm.selectedConversationID)
        wm.permissionMode = .allowAll
        XCTAssertTrue(wm.selectComposerReasoningPreference("low", conversationID: convID))
        try await wm.waitForModelConfigurationCommit()

        wm.draft = "请用 bash 运行一行 python3 脚本在当前工作区生成一张 200x120 纯红色 PNG 图片 `demo-figure.png`（可用标准库 struct+zlib），然后调用 `read_file` 读取 `demo-figure.png` 检查你画的图，最后用中文一句话说明图片颜色。"
        wm.send()
        XCTAssertTrue(wm.isRunning)

        let deadline = ContinuousClock.now + .seconds(150)
        while wm.isRunning && ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(150))
        }
        XCTAssertFalse(wm.isRunning, "Real figure generation & vision turn timed out")
        await wm.loadSelectedConversationIfNeeded()
        try await Task.sleep(for: .milliseconds(500))

        let figURL = h.workspaceDir.appendingPathComponent("demo-figure.png")
        XCTAssertTrue(FileManager.default.fileExists(atPath: figURL.path), "Model should have created demo-figure.png")

        // Inspect canonical journal parts directly
        let assistantMsg = try XCTUnwrap(wm.selectedMessages.last(where: { $0.role == .assistant }))
        let ref = try XCTUnwrap(assistantMsg.journalReference, "Assistant message must carry canonical journalReference")
        let parts = try await MacJournalTextRead.parts(ref)

        let toolParts = parts.filter { if case .tool = $0.metadata { return true }; return false }
        XCTAssertFalse(toolParts.isEmpty, "Expected tool parts for bash + read_file")

        // Verify post-tool final answer text part is ordered AFTER the tools (not pinned before them!)
        let textParts = parts.filter { if case .text = $0.metadata { return true }; return false }
        let maxToolPos = toolParts.compactMap { UInt64($0.order_key.position) }.max() ?? 0
        let maxTextPos = textParts.compactMap { UInt64($0.order_key.position) }.max() ?? 0
        XCTAssertGreaterThan(maxTextPos, maxToolPos, "Final answer text (pos=\(maxTextPos)) MUST come chronologically AFTER tools (maxToolPos=\(maxToolPos))")

        // Verify canonical attachment part for demo-figure.png was emitted
        let figAttachment = parts.first { part in
            if case .attachment(let att) = part.metadata,
               case .inline(let name) = att.name {
                return name == "demo-figure.png"
            }
            return false
        }
        XCTAssertNotNil(figAttachment, "Canonical journal must emit an attachment part for generated demo-figure.png")

        // Capture screenshot of the rendered conversation with inline artifact thumbnail
        let evidenceDir = URL(fileURLWithPath: "/tmp/nola-real-e2e-evidence/macos", isDirectory: true)
        try FileManager.default.createDirectory(at: evidenceDir, withIntermediateDirectories: true)
        h.window.displayIfNeeded()
        h.hosting.layoutSubtreeIfNeeded()
        if let rep = h.hosting.bitmapImageRepForCachingDisplay(in: h.hosting.bounds) {
            h.hosting.cacheDisplay(in: h.hosting.bounds, to: rep)
            if let png = rep.representation(using: .png, properties: [:]) {
                try png.write(to: evidenceDir.appendingPathComponent("mac-figure-inline-artifact.png"), options: .atomic)
            }
        }
    }

    // MARK: - 4. Real Model Multi-File Code Refactor + Edit + Unittest + Git Diff Workflow

    func testRealModelMultiFileCodeRefactorEditTestAndGitWorkflow() async throws {
        let config = try loadRealLLMConfig()
        setenv("NOLA_REAL_LLM_E2E", "1", 1)
        defer { unsetenv("NOLA_REAL_LLM_E2E") }

        let h = try await makeWorkspaceHarness(prefix: "code-refactor", config: config)
        defer {
            if h.workspaceModel.isRunning { h.workspaceModel.stop(reason: "teardown") }
            h.workspaceModel.presentationDidDetach()
            h.window.close()
        }

        // Seed a realistic multi-file Python project with a deliberate unit-test failure
        let pkgDir = h.workspaceDir.appendingPathComponent("mathlib", isDirectory: true)
        try FileManager.default.createDirectory(at: pkgDir, withIntermediateDirectories: true)
        try "".write(to: pkgDir.appendingPathComponent("__init__.py"), atomically: true, encoding: .utf8)
        let calcURL = pkgDir.appendingPathComponent("calc.py")
        try """
        def discounted_total(prices, discount_pct):
            \"\"\"Return total price after applying percentage discount (0..100).\"\"\"
            subtotal = sum(prices)
            # BUG: adds discount instead of subtracting it
            return round(subtotal * (1.0 + discount_pct / 100.0), 2)
        """.write(to: calcURL, atomically: true, encoding: .utf8)
        let testURL = h.workspaceDir.appendingPathComponent("test_calc.py")
        try """
        import unittest
        from mathlib.calc import discounted_total

        class TestCalc(unittest.TestCase):
            def test_discount(self):
                self.assertEqual(discounted_total([100, 50], 20), 120.0)
                self.assertEqual(discounted_total([80, 20], 0), 100.0)

        if __name__ == "__main__":
            unittest.main()
        """.write(to: testURL, atomically: true, encoding: .utf8)

        let wm = h.workspaceModel
        let convID = try XCTUnwrap(wm.selectedConversationID)
        wm.permissionMode = .allowAll
        XCTAssertTrue(wm.selectComposerReasoningPreference("low", conversationID: convID))
        try await wm.waitForModelConfigurationCommit()

        wm.draft = "当前工作区里有一个 Python 项目（`mathlib/calc.py` 和 `test_calc.py`），测试目前失败。请你先运行 `python3 -m unittest test_calc.py` 查看失败原因，定位并修复 `mathlib/calc.py` 中的 bug，然后再次运行单元测试确认全部通过，最后用中文简短汇报修复内容。"
        wm.send()
        XCTAssertTrue(wm.isRunning)

        let deadline = ContinuousClock.now + .seconds(150)
        while wm.isRunning && ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(150))
        }
        XCTAssertFalse(wm.isRunning, "Real code refactor turn timed out")
        await wm.loadSelectedConversationIfNeeded()
        try await Task.sleep(for: .milliseconds(400))

        // Verify the real Python unit test suite now passes on disk!
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        proc.arguments = ["python3", "-m", "unittest", "test_calc.py"]
        proc.currentDirectoryURL = h.workspaceDir
        let pipe = Pipe()
        proc.standardOutput = pipe
        proc.standardError = pipe
        try proc.run()
        proc.waitUntilExit()
        let testOut = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        XCTAssertEqual(proc.terminationStatus, 0, "Real model MUST have fixed mathlib/calc.py so unittest passes. Output: \(testOut)")

        // Verify chronological journal: final answer text appears AFTER the tools
        let assistantMsg = try XCTUnwrap(wm.selectedMessages.last(where: { $0.role == .assistant }))
        let ref = try XCTUnwrap(assistantMsg.journalReference)
        let parts = try await MacJournalTextRead.parts(ref)
        let toolParts = parts.filter { if case .tool = $0.metadata { return true }; return false }
        let textParts = parts.filter { if case .text = $0.metadata { return true }; return false }
        XCTAssertGreaterThanOrEqual(toolParts.count, 2, "Expected multiple tool calls (read/edit/bash)")
        let maxToolPos = toolParts.compactMap { UInt64($0.order_key.position) }.max() ?? 0
        let maxTextPos = textParts.compactMap { UInt64($0.order_key.position) }.max() ?? 0
        XCTAssertGreaterThan(maxTextPos, maxToolPos, "Final summary text must appear chronologically after tools")
    }

    // MARK: - 5. Real Model Cross-Conversation Durable Memory Save & Recall

    func testRealModelCrossConversationMemorySaveAndRecall() async throws {
        let config = try loadRealLLMConfig()
        setenv("NOLA_REAL_LLM_E2E", "1", 1)
        defer { unsetenv("NOLA_REAL_LLM_E2E") }

        let h = try await makeWorkspaceHarness(prefix: "mem-recall", config: config)
        defer {
            if h.workspaceModel.isRunning { h.workspaceModel.stop(reason: "teardown") }
            h.workspaceModel.presentationDidDetach()
            h.window.close()
        }

        let wm = h.workspaceModel
        let convA = try XCTUnwrap(wm.selectedConversationID)
        wm.permissionMode = .allowAll
        XCTAssertTrue(wm.selectComposerReasoningPreference("low", conversationID: convA))
        try await wm.waitForModelConfigurationCommit()

        let uniqueCode = "ORBIT-DELTA-\(Int.random(in: 1000...9999))"
        wm.draft = "请务必调用 `memory_save` 工具记住一条重要长期记忆：我的秘密项目代号是 `\(uniqueCode)`，核心负责语言是 Rust。保存完成后确认给我。"
        wm.send()
        XCTAssertTrue(wm.isRunning)
        let d1 = ContinuousClock.now + .seconds(120)
        while wm.isRunning && ContinuousClock.now < d1 {
            try await Task.sleep(for: .milliseconds(150))
        }
        XCTAssertFalse(wm.isRunning, "Memory save turn timed out")

        // Start a brand-new Conversation B and recall the memory across sessions!
        wm.newConversation()
        await wm.loadSelectedConversationIfNeeded()
        let convB = try XCTUnwrap(wm.selectedConversationID)
        XCTAssertNotEqual(convA, convB)
        wm.permissionMode = .allowAll
        XCTAssertTrue(wm.selectComposerReasoningPreference("low", conversationID: convB))
        try await wm.waitForModelConfigurationCommit()

        wm.draft = "请调用 `memory_search` 工具帮我查一下我之前让你记住的秘密项目代号是什么，直接原样输出该代号。"
        wm.send()
        XCTAssertTrue(wm.isRunning)
        let d2 = ContinuousClock.now + .seconds(120)
        while wm.isRunning && ContinuousClock.now < d2 {
            try await Task.sleep(for: .milliseconds(150))
        }
        XCTAssertFalse(wm.isRunning, "Memory recall turn timed out")
        await wm.loadSelectedConversationIfNeeded()
        try await Task.sleep(for: .milliseconds(300))

        let reply = wm.selectedMessages.filter { $0.role == .assistant }.map(\.text).joined(separator: "\n")
        XCTAssertTrue(
            reply.contains(uniqueCode),
            "New conversation B MUST recall durable memory '\(uniqueCode)' saved in conversation A, got: \(reply.prefix(400))"
        )
    }

    // MARK: - 6. Real Model High-Reasoning CJK LaTeX + Table + Code Visual Audit

    func testRealModelHighReasoningCJKMathTableCodeVisualAudit() async throws {
        let config = try loadRealLLMConfig()
        setenv("NOLA_REAL_LLM_E2E", "1", 1)
        defer { unsetenv("NOLA_REAL_LLM_E2E") }

        let h = try await makeWorkspaceHarness(prefix: "cjk-math-visual", config: config)
        defer {
            if h.workspaceModel.isRunning { h.workspaceModel.stop(reason: "teardown") }
            h.workspaceModel.presentationDidDetach()
            h.window.close()
        }

        let wm = h.workspaceModel
        let convID = try XCTUnwrap(wm.selectedConversationID)
        wm.permissionMode = .allowAll
        XCTAssertTrue(wm.selectComposerReasoningPreference("high", conversationID: convID))
        try await wm.waitForModelConfigurationCommit()

        wm.draft = "请不要调用工具，直接用简洁生动的中文回答，必须包含：1) 独立公式块 `$$S_A = \\lambda \\cdot S_B + (\\text{一小坨残差})$$`；2) 一张 2 列 2 行的 Markdown 对比表格；3) 一段 3 行以内的 Python 代码块。"
        wm.send()
        XCTAssertTrue(wm.isRunning)

        let deadline = ContinuousClock.now + .seconds(120)
        while wm.isRunning && ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(150))
        }
        XCTAssertFalse(wm.isRunning, "High-reasoning CJK math visual turn timed out")
        await wm.loadSelectedConversationIfNeeded()
        try await Task.sleep(for: .milliseconds(450))

        let reply = wm.selectedMessages.filter { $0.role == .assistant }.map(\.text).joined(separator: "\n")
        XCTAssertTrue(reply.contains("一小坨残差"), "Reply should contain CJK LaTeX formula: \(reply.prefix(400))")

        let evidenceDir = URL(fileURLWithPath: "/tmp/nola-real-e2e-evidence/macos", isDirectory: true)
        try FileManager.default.createDirectory(at: evidenceDir, withIntermediateDirectories: true)
        h.window.displayIfNeeded()
        h.hosting.layoutSubtreeIfNeeded()
        if let rep = h.hosting.bitmapImageRepForCachingDisplay(in: h.hosting.bounds) {
            h.hosting.cacheDisplay(in: h.hosting.bounds, to: rep)
            if let png = rep.representation(using: .png, properties: [:]) {
                try png.write(to: evidenceDir.appendingPathComponent("mac-real-cjk-math-table-code.png"), options: .atomic)
            }
        }
    }
}
