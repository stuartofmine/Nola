// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Runs a real-provider, multi-turn Agent workflow through the production Mac chat UI.

import AppKit
import Foundation
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacRealProductUIEndToEndTests: XCTestCase {
    /// Opt-in real acceptance. The supplied mirror must come from the same
    /// actual paired submission; this test never manufactures task projections.
    func testRealTaskGuidanceLostResponseKeepsOriginalIntent() async throws {
        let env = ProcessInfo.processInfo.environment
        guard let endpoint = env["NOLA_MAC_REAL_HOST_ENDPOINT"],
              let tokenPath = env["NOLA_MAC_REAL_HOST_TOKEN_FILE"],
              let sourceDB = env["NOLA_MAC_REAL_HOST_DATABASE"],
              let mirror = env["NOLA_MAC_REAL_GUIDANCE_MIRROR"],
              let evidencePath = env["NOLA_MAC_REAL_GUIDANCE_EVIDENCE"],
              let proxyLog = env["NOLA_MAC_REAL_GUIDANCE_PROXY_LOG"] else {
            throw XCTSkip("Requires an isolated, genuinely active paired child and its original mirror")
        }
        guard env["NOLA_TEST_MODE"] == "1", env["NOLA_USE_REAL_KEYCHAIN_IN_TESTS"] == "0",
              mirror.hasPrefix("/tmp/nola-"), evidencePath.hasPrefix("/tmp/nola-") else {
            throw MacApplicationPortError.invalidResponse
        }
        let evidence = URL(fileURLWithPath: evidencePath)
        try FileManager.default.createDirectory(at: evidence, withIntermediateDirectories: true)
        func quote(_ value: String) -> String { "'" + value.replacingOccurrences(of: "'", with: "''") + "'" }
        func query(_ sql: String, database: String? = nil) throws -> [[String: Any]] {
            let process = Process(); process.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
            process.arguments = ["-readonly", "-json", database ?? sourceDB, sql]
            let pipe = Pipe(); process.standardOutput = pipe
            try process.run(); let bytes = pipe.fileHandleForReading.readDataToEndOfFile(); process.waitUntilExit()
            guard process.terminationStatus == 0 else { throw MacApplicationPortError.invalidResponse }
            return bytes.isEmpty ? [] : try XCTUnwrap(JSONSerialization.jsonObject(with: bytes) as? [[String: Any]])
        }
        let port = try MacApplicationPort(databaseURL: URL(fileURLWithPath: mirror))
        let store = MacRemoteHostConfigurationStore(applicationPort: port)
        let service = MacRemoteHostService(configurationStore: store)
        let token = try String(contentsOfFile: tokenPath, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
        let url = try XCTUnwrap(URL(string: endpoint))
        let report = try await service.testConnection(configuration: .init(baseURL: url, token: token))
        try store.save(url: endpoint, token: token, hostID: report.hostID)
        let repository = MacCanonicalConversationRepository(store: port)
        let existingOwner = env["NOLA_MAC_REAL_GUIDANCE_CONVERSATION"]
        let model = MacWorkspaceModel(initialConversationID: existingOwner, canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: service, modelApplication: port,
            readinessIssue: { location in
                location == .connectedHost && store.load() != nil ? nil : MacReadinessService.blockingIssue(for: location)
            })
        model.updateRemoteHostConfigurationState(true)
        let sourceOwner: (conversation: String, run: String, assistant: String)?
        if env["NOLA_MAC_REAL_GUIDANCE_RENDER_ONLY"] == "1" {
            let conversation = try XCTUnwrap(existingOwner)
            let run = try XCTUnwrap(env["NOLA_MAC_REAL_GUIDANCE_PARENT_RUN"])
            let terminal = try XCTUnwrap(port.canonicalRunTerminal(conversationID: conversation, runID: run))
            sourceOwner = (conversation, run, terminal.assistantMessageID)
        } else if let prior = try port.remoteSubmissionIntent(conversationID: existingOwner ?? model.selectedConversationID ?? ""),
                  prior.phase == .admitted {
            sourceOwner = (prior.identity.conversation_id, prior.identity.submission_id, prior.identity.assistant_message_id)
        } else { sourceOwner = nil }
        if let sourceOwner {
            let batch = try await service.observeRun(conversationID: sourceOwner.conversation,
                expectedRunID: sourceOwner.run, assistantMessageID: sourceOwner.assistant,
                target: service.captureSubmissionTarget())
            if env["NOLA_MAC_REAL_GUIDANCE_RENDER_ONLY"] == "1" {
                let usage = try XCTUnwrap(batch.observation?.context_usage)
                XCTAssertNotNil(usage.usage.prompt_cache_diagnostic)
            }
            if let sourceUsage = batch.observation?.context_usage {
                let row = try XCTUnwrap(query("SELECT record_json FROM remote_run_context_usage WHERE run_id=\(quote(sourceUsage.identity.run_id)) AND source_host_id=\(quote(report.hostID));", database: mirror).first)
                let storedJSON = try XCTUnwrap(row["record_json"] as? String)
                let stored = try JSONDecoder().decode(MacCommittedRunContextUsage.self, from: Data(storedJSON.utf8))
                XCTAssertEqual(stored.record_digest, sourceUsage.record_digest)
                XCTAssertEqual(stored, sourceUsage)
                if let diagnostic = stored.usage.prompt_cache_diagnostic {
                    let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(diagnostic)) as? [String: Any])
                    XCTAssertNotNil(object["schemaVersion"])
                    XCTAssertNil(object["schema_version"])
                }
                try JSONSerialization.data(withJSONObject: ["source_record_digest": sourceUsage.record_digest,
                    "stored_record_digest": stored.record_digest,
                    "has_prompt_cache_diagnostic": stored.usage.prompt_cache_diagnostic != nil], options: [.prettyPrinted])
                    .write(to: evidence.appendingPathComponent("source-context-usage-roundtrip.json"))
            }
        }
        await model.loadSelectedConversationIfNeeded()
        model.presentationDidAttach()
        defer { model.presentationDidDetach() }
        if env["NOLA_MAC_REAL_GUIDANCE_TASK"] == nil {
            if existingOwner == nil { model.newConversation() }
            model.updateRemoteHostConfigurationState(true)
            model.updateRemoteModelCatalog(try await service.fetchRemoteModelCatalog())
            model.executionHost = .remote
            try await model.waitForModelConfigurationCommit()
            model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
            try await model.waitForModelConfigurationCommit()
            model.draft = "Delegate exactly one task using spawn_agent with the explore profile. The child must first call request_user_input to ask whether to proceed with multiplying 17 × 23, and wait for the user response before calculating. Do not answer the child question yourself. No files or network searches are needed."
            model.send()
            guard model.isRunning else {
                throw NSError(domain: "RealGuidanceUI", code: 1, userInfo: [NSLocalizedDescriptionKey:
                    "Submission did not start: selected=\(model.selectedConversationID ?? "nil") activity=\(model.activity) draft=\(model.draft.count) pending=\(model.selectedModelConfigurationPending)"])
            }
        }
        let owner = try XCTUnwrap(model.selectedConversationID)
        try JSONSerialization.data(withJSONObject: ["conversation_id": owner, "mirror": mirror], options: [.prettyPrinted])
            .write(to: evidence.appendingPathComponent("submission-owner.json"))
        let taskDeadline = Date().addingTimeInterval(180)
        var sourceTask: [String: Any]?
        repeat {
            let taskFilter = env["NOLA_MAC_REAL_GUIDANCE_TASK"].map { "id=" + quote($0) }
                ?? "status!='sealed'"
            sourceTask = try query("SELECT id,status,owner_session_id,title FROM durable_tasks WHERE owner_session_id=\(quote(owner)) AND \(taskFilter) ORDER BY created_at_ms DESC LIMIT 1;").first
            if sourceTask != nil { break }
            if !model.isRunning {
                throw NSError(domain: "RealGuidanceUI", code: 2, userInfo: [NSLocalizedDescriptionKey:
                    "Source did not admit a child: activity=\(model.activity), recovery=\(model.remoteRecoveryError ?? "none")"])
            }
            try await Task.sleep(for: .seconds(1))
        } while Date() < taskDeadline
        let task = try XCTUnwrap(sourceTask, "Real parent did not produce an active child")
        let taskID = try XCTUnwrap(task["id"] as? String)
        XCTAssertEqual(task["owner_session_id"] as? String, owner)
        let messageDeadline = Date().addingTimeInterval(30)
        var taskMessage: MacChatMessage?
        repeat {
            await model.loadSelectedConversationIfNeeded()
            for candidate in model.messagesByConversation[owner] ?? [] {
                guard let reference = candidate.journalReference else { continue }
                let parts = try await MacJournalTextRead.parts(reference)
                if parts.contains(where: {
                    if case .task(let value) = $0.metadata { return value.task_id == taskID }
                    return false
                }) { taskMessage = candidate; break }
            }
            if taskMessage != nil { break }
            try await Task.sleep(for: .milliseconds(200))
        } while Date() < messageDeadline
        let message = try XCTUnwrap(taskMessage, "Original task must be present in its real canonical journal")
        let parent = try XCTUnwrap(message.journalReference?.cursor.identity.run_id)
        try JSONSerialization.data(withJSONObject: ["conversation_id": owner, "parent_run_id": parent,
            "task_id": taskID, "mirror": mirror], options: [.prettyPrinted, .sortedKeys])
            .write(to: evidence.appendingPathComponent("owner.json"))
        let accessibility = try MacHostedAccessibilitySession(); defer { accessibility.restore() }
        let hosting = NSHostingView(rootView: MacMessageView(message: message, transcriptWidth: 900,
            isStreaming: false, liveActivities: [], isSearchMatch: false,
            onSendTaskGuidance: { task, text in
                model.sendTaskGuidance(conversationID: owner, parentRunID: parent, taskID: task, message: text)
            }, onRequestTaskCancellation: { _ in XCTFail("Guidance acceptance must not cancel the task") }).frame(width: 900))
        hosting.frame = NSRect(x: 0, y: 0, width: 900, height: 900)
        let window = NSWindow(contentRect: hosting.bounds, styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.contentView = hosting; window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; model.selectedConversationID = nil }
        func elements() -> [AnyObject] {
            var queue: [AnyObject] = [hosting]; var seen = Set<ObjectIdentifier>(); var result: [AnyObject] = []
            while !queue.isEmpty, seen.count < 4_000 {
                let item = queue.removeFirst(); guard seen.insert(ObjectIdentifier(item)).inserted else { continue }
                result.append(item)
                queue += ((item.accessibilityChildren?() ?? nil) ?? []).map { $0 as AnyObject }
                if let view = item as? NSView { queue += view.subviews }
            }
            return result
        }
        await settle(hosting)
        // Task Parts are explicit flow boundaries and remain directly reachable;
        // a Reasoning/Tool process disclosure does not own task controls.
        let cardDeadline = Date().addingTimeInterval(15)
        while accessibilityElement(hosting, identifier: "chat.journal.task") == nil, Date() < cardDeadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        try accessibilityTree(hosting).write(to: evidence.appendingPathComponent("before-guidance-accessibility.txt"), atomically: true, encoding: .utf8)
        XCTAssertNotNil(accessibilityElement(hosting, identifier: "chat.journal.task"))
        if env["NOLA_MAC_REAL_GUIDANCE_RENDER_ONLY"] == "1" {
            try capturePNG(hosting, to: evidence.appendingPathComponent("original-child-cold-render.png"))
            try accessibilityTree(hosting).write(to: evidence.appendingPathComponent("original-child-cold-render.txt"), atomically: true, encoding: .utf8)
            return
        }
        let field = try XCTUnwrap(elements().compactMap { $0 as? NSTextField }.first {
            $0.placeholderString == String(localized: "Task guidance", table: "Chat")
        })
        let marker = "NOLA-GUIDANCE-" + UUID().uuidString
        let instruction = "Keep the current task scope. Include \(marker) in the final status."
        field.selectText(nil)
        let editor = try XCTUnwrap(field.currentEditor() as? NSTextView)
        editor.insertText(instruction, replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
        window.makeFirstResponder(hosting); await settle(hosting)
        let (_, armResponse) = try await URLSession.shared.data(from: url.appendingPathComponent("__test/drop-next-guidance-response"))
        XCTAssertEqual((armResponse as? HTTPURLResponse)?.statusCode, 200)
        let send = try XCTUnwrap(elements().first {
            ($0.accessibilityRole?() ?? nil) == .button &&
                ($0.accessibilityLabel?() ?? nil) == String(localized: "Task guidance", table: "Chat")
        })
        XCTAssertEqual(send.accessibilityPerformPress?(), true)
        let sql = "SELECT message_id,record_json FROM runtime_task_guidance WHERE task_id=\(quote(taskID)) AND instr(record_json,\(quote(marker)))>0;"
        let deadline = Date().addingTimeInterval(45)
        var receipts: [[String: Any]] = []
        repeat {
            receipts = try query(sql)
            if !receipts.isEmpty { break }
            try await Task.sleep(for: .milliseconds(200))
        } while Date() < deadline
        XCTAssertEqual(receipts.count, 1)
        let receiptJSON = try XCTUnwrap(receipts.first?["record_json"] as? String)
        let receipt = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(receiptJSON.utf8)) as? [String: Any])
        XCTAssertEqual(receipt["content"] as? String, instruction)
        XCTAssertEqual(receipt["owner_session_id"] as? String, owner)
        XCTAssertNotEqual(receipt["status"] as? String, "rejected", "A rejected instruction is not successful guidance")
        try JSONSerialization.data(withJSONObject: receipts, options: [.prettyPrinted, .sortedKeys])
            .write(to: evidence.appendingPathComponent("source-guidance.json"))
        try await Task.sleep(for: .seconds(2))
        try capturePNG(hosting, to: evidence.appendingPathComponent("guidance-after-response-loss.png"))
        try accessibilityTree(hosting).write(to: evidence.appendingPathComponent("guidance-accessibility.txt"), atomically: true, encoding: .utf8)
        // A second workspace reload is a cold observer, never another send.
        model.selectedConversationID = nil
        let coldPort = try MacApplicationPort(databaseURL: URL(fileURLWithPath: mirror))
        let coldService = MacRemoteHostService(configurationStore: MacRemoteHostConfigurationStore(applicationPort: coldPort))
        let cold = MacWorkspaceModel(initialConversationID: owner,
            canonicalRepository: MacCanonicalConversationRepository(store: coldPort),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: coldService, modelApplication: coldPort)
        await cold.loadSelectedConversationIfNeeded()
        try await Task.sleep(for: .seconds(2))
        XCTAssertEqual(try query(sql).count, 1)
        cold.selectedConversationID = nil
        let messageID = try XCTUnwrap(receipts.first?["message_id"] as? String)
        let events = try String(contentsOfFile: proxyLog, encoding: .utf8).split(separator: "\n").compactMap {
            try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any]
        }.filter { $0["message_id"] as? String == messageID }
        XCTAssertEqual(events.filter { $0["event"] as? String == "guidance_response" }.count, 1,
            "Positive authoritative receipt must not cause a second instruction POST")
        XCTAssertFalse(events.filter { $0["event"] as? String == "guidance_receipt_read" }.isEmpty,
            "Lost response must reconcile the original message ID against source")
        XCTAssertTrue(events.allSatisfy { $0["parent_run_id"] as? String == parent && $0["task_id"] as? String == taskID })
        try JSONSerialization.data(withJSONObject: events, options: [.prettyPrinted, .sortedKeys])
            .write(to: evidence.appendingPathComponent("transport-audit.json"))
    }

    func testRealPairedGoalControlsAndColdSourceWindow() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard let endpoint = environment["NOLA_MAC_REAL_HOST_ENDPOINT"],
              let tokenPath = environment["NOLA_MAC_REAL_HOST_TOKEN_FILE"],
              let sourceDatabase = environment["NOLA_MAC_REAL_HOST_DATABASE"],
              let evidencePath = environment["NOLA_MAC_REAL_GOAL_EVIDENCE"] else {
            throw XCTSkip("Requires an explicitly configured isolated current Host")
        }
        guard environment["NOLA_TEST_MODE"] == "1", environment["NOLA_USE_REAL_KEYCHAIN_IN_TESTS"] == "0" else {
            throw MacApplicationPortError.invalidResponse
        }
        let evidence = URL(fileURLWithPath: evidencePath)
        guard evidence.path.hasPrefix("/tmp/nola-") else { throw MacApplicationPortError.invalidResponse }
        try FileManager.default.createDirectory(at: evidence, withIntermediateDirectories: true)
        let token = try String(contentsOfFile: tokenPath, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
        let existingOwner = environment["NOLA_MAC_REAL_EXISTING_GOAL_CONVERSATION"]
        let databaseURL = environment["NOLA_MAC_REAL_EXISTING_GOAL_MIRROR"].map { URL(fileURLWithPath: $0) }
            ?? evidence.appendingPathComponent("mirror.db")
        guard databaseURL.path.hasPrefix("/tmp/nola-") else { throw MacApplicationPortError.invalidResponse }
        let port = try MacApplicationPort(databaseURL: databaseURL)
        let store = MacRemoteHostConfigurationStore(applicationPort: port)
        let service = MacRemoteHostService(configurationStore: store)
        let url = try XCTUnwrap(URL(string: endpoint))
        let report = try await service.testConnection(configuration: .init(baseURL: url, token: token))
        try store.save(url: endpoint, token: token, hostID: report.hostID)
        let catalog = try await service.fetchRemoteModelCatalog()
        let repository = MacCanonicalConversationRepository(store: port)
        let model = MacWorkspaceModel(initialConversationID: existingOwner, canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: service)
        if existingOwner == nil { model.newConversation() }
        let owner = try XCTUnwrap(model.selectedConversationID)
        model.updateRemoteHostConfigurationState(true)
        model.updateRemoteModelCatalog(catalog)
        if existingOwner == nil {
            model.executionHost = .remote
            try await model.waitForModelConfigurationCommit()
            let option = try XCTUnwrap(model.configuredModelOptions.first)
            model.selectModel(option)
            try await model.waitForModelConfigurationCommit()
        }
        let selectedModelID = try XCTUnwrap(model.selectedConversation?.modelSelectionID)
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore(); model.selectedConversationID = nil }
        let hosting = NSHostingView(rootView: MacInspectorProgressView(model: model, mode: .goal).padding(20).frame(width: 480))
        hosting.frame = NSRect(x: 0, y: 0, width: 480, height: 900)
        let window = NSWindow(contentRect: hosting.bounds, styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.contentView = hosting; window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil }
        func wait(_ identifier: String, seconds: Double = 60) async throws -> AnyObject {
            let deadline = Date().addingTimeInterval(seconds)
            while Date() < deadline {
                if let element = accessibilityElement(hosting, identifier: identifier) { return element }
                try await Task.sleep(for: .milliseconds(100))
            }
            try? capturePNG(hosting, to: evidence.appendingPathComponent("timeout-\(identifier).png"))
            try? accessibilityTree(hosting).write(to: evidence.appendingPathComponent("timeout-\(identifier).txt"), atomically: true, encoding: .utf8)
            throw NSError(domain: "RealGoalUI", code: 1, userInfo: [NSLocalizedDescriptionKey: "Missing \(identifier): \(accessibilityTree(hosting))"])
        }
        let escapedOwner = owner.replacingOccurrences(of: "'", with: "''")
        func sourceQuery(_ sql: String) throws -> Data {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
            process.arguments = ["-readonly", "-json", sourceDatabase, sql]
            let output = Pipe(); process.standardOutput = output
            try process.run()
            let bytes = output.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            guard process.terminationStatus == 0 else { throw MacApplicationPortError.invalidResponse }
            return bytes
        }
        func waitForRealModelRuns(_ expected: Int) async throws {
            let deadline = Date().addingTimeInterval(180)
            while Date() < deadline {
                let bytes = try sourceQuery("SELECT count(DISTINCT e.run_id) AS count FROM run_events e JOIN runs r ON r.id=e.run_id WHERE r.conversation_id='\(escapedOwner)' AND e.event_kind='part_updated' AND json_extract(e.payload_json,'$.update.type') IN ('text_delta','reasoning_delta') AND length(coalesce(json_extract(e.payload_json,'$.update.delta'),json_extract(e.payload_json,'$.update.detail_delta')))>0;")
                let rows = try JSONSerialization.jsonObject(with: bytes) as? [[String: Int]]
                if (rows?.first?["count"] ?? 0) >= expected { return }
                try await Task.sleep(for: .seconds(1))
            }
            throw NSError(domain: "RealGoalUI", code: 2, userInfo: [NSLocalizedDescriptionKey: "No real model text delta for run \(expected)"])
        }
        let initialCountData = try sourceQuery("SELECT count(DISTINCT e.run_id) AS count FROM run_events e JOIN runs r ON r.id=e.run_id WHERE r.conversation_id='\(escapedOwner)' AND e.event_kind='part_updated' AND json_extract(e.payload_json,'$.update.type') IN ('text_delta','reasoning_delta') AND length(coalesce(json_extract(e.payload_json,'$.update.delta'),json_extract(e.payload_json,'$.update.detail_delta')))>0;")
        let initialModelRunCount = (try JSONSerialization.jsonObject(with: initialCountData) as? [[String: Int]])?.first?["count"] ?? 0
        if existingOwner != nil {
            let resume = try await wait("mac-goal-resume")
            guard resume.accessibilityPerformPress?() == true else { throw MacApplicationPortError.invalidResponse }
        } else {
            _ = try await wait("mac-goal-objective")
            func goalField(in view: NSView) -> NSTextField? {
                if let field = view as? NSTextField, field.placeholderString == "Goal" { return field }
                return view.subviews.lazy.compactMap { goalField(in: $0) }.first
            }
            let field = try XCTUnwrap(goalField(in: hosting))
            let objective = "Write 40 detailed numbered explanations of software verification, each with a worked example. Complete all 40 before finishing this goal."
            field.selectText(nil)
            let editor = try XCTUnwrap(field.currentEditor() as? NSTextView)
            editor.insertText(objective, replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
            window.makeFirstResponder(hosting)
            await settle(hosting)
            XCTAssertEqual(field.stringValue, objective)
            let startButton = try await wait("mac-goal-start")
            guard startButton.accessibilityPerformPress?() == true else {
                try capturePNG(hosting, to: evidence.appendingPathComponent("start-unavailable.png"))
                throw MacApplicationPortError.invalidResponse
            }
        }
        _ = try await wait("mac-goal-pause", seconds: 120)
        try await waitForRealModelRuns(initialModelRunCount + 1)
        let pauseButton = try await wait("mac-goal-pause")
        if environment["NOLA_MAC_REAL_HOST_FAULT_PROXY"] == "1" {
            let (_, response) = try await URLSession.shared.data(from: url.appendingPathComponent("__test/drop-next-goal-response"))
            XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        }
        XCTAssertEqual(pauseButton.accessibilityPerformPress?(), true)
        _ = try await wait("mac-goal-resume", seconds: 120)
        let pausedSnapshot = try await model.inspectorGoalSnapshot(conversationID: owner)
        let paused = try XCTUnwrap(pausedSnapshot)
        XCTAssertEqual(paused.active?.status, "paused")
        try capturePNG(hosting, to: evidence.appendingPathComponent("paused.png"))
        let resumeButton = try await wait("mac-goal-resume")
        XCTAssertEqual(resumeButton.accessibilityPerformPress?(), true)
        _ = try await wait("mac-goal-pause", seconds: 120)
        try await waitForRealModelRuns(initialModelRunCount + 2)
        let cancelButton = try await wait("mac-goal-cancel")
        XCTAssertEqual(cancelButton.accessibilityPerformPress?(), true)
        _ = try await wait("mac-goal-start", seconds: 120)
        let finalSnapshot = try await model.inspectorGoalSnapshot(conversationID: owner)
        let final = try XCTUnwrap(finalSnapshot)
        XCTAssertEqual(final.active?.status, "cancelled")
        try capturePNG(hosting, to: evidence.appendingPathComponent("cancelled.png"))
        XCTAssertNotNil(final.applied_operations, "The source must retain exact control receipts")
        let terminalDeadline = Date().addingTimeInterval(60)
        var sourceRuns: [[String: Any]] = []
        repeat {
            let data = try sourceQuery("SELECT r.id,r.status,(SELECT count(*) FROM run_events e WHERE e.run_id=r.id AND e.event_kind='part_updated' AND json_extract(e.payload_json,'$.update.type')='text_delta') AS model_text_deltas,(SELECT count(*) FROM run_events e WHERE e.run_id=r.id AND e.event_kind='part_updated' AND json_extract(e.payload_json,'$.update.type')='reasoning_delta') AS model_reasoning_deltas FROM runs r WHERE r.conversation_id='\(escapedOwner)' ORDER BY r.created_at_ms;")
            sourceRuns = (try JSONSerialization.jsonObject(with: data) as? [[String: Any]]) ?? []
            if sourceRuns.count >= 2 && sourceRuns.allSatisfy({ $0["status"] as? String == "sealed" }) { break }
            try await Task.sleep(for: .milliseconds(500))
        } while Date() < terminalDeadline
        XCTAssertGreaterThanOrEqual(sourceRuns.count, 2)
        XCTAssertTrue(sourceRuns.allSatisfy { $0["status"] as? String == "sealed" })
        try JSONSerialization.data(withJSONObject: sourceRuns, options: [.prettyPrinted, .sortedKeys])
            .write(to: evidence.appendingPathComponent("source-runs.json"))
        let coldPort = try MacApplicationPort(databaseURL: port.databaseURL)
        let cold = MacWorkspaceModel(initialConversationID: owner,
            canonicalRepository: MacCanonicalConversationRepository(store: coldPort),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: service)
        defer { cold.selectedConversationID = nil }
        let coldSnapshot = try await cold.inspectorGoalSnapshot(conversationID: owner)
        XCTAssertEqual(coldSnapshot?.active?.id, final.active?.id)
        XCTAssertEqual(coldSnapshot?.active?.status, "cancelled")
        hosting.rootView = MacInspectorProgressView(model: cold, mode: .goal).padding(20).frame(width: 480)
        _ = try await wait("mac-goal-start")
        try capturePNG(hosting, to: evidence.appendingPathComponent("cold-cancelled.png"))
        let saved: [String: Any] = ["conversation_id":owner, "host_id":report.hostID,
            "goal_id":final.active?.id ?? "", "status":final.active?.status ?? "", "model_selection":selectedModelID,
            "mirror_database":port.databaseURL.path,
            "applied_operation_ids": final.applied_operations.map { Array($0.keys).sorted() } ?? []]
        try JSONSerialization.data(withJSONObject: saved, options: [.prettyPrinted, .sortedKeys])
            .write(to: evidence.appendingPathComponent("report.json"))
    }

    func testOrdinaryComposerMentionSendsRealExpertReply() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1", let config = realLLMConfiguration(environment) else {
            throw XCTSkip("Requires isolated real-provider XCTest runner")
        }
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let database = MacApplicationPort.defaultDatabaseURL
        guard MacAppEnvironment.contains(database, in: URL(fileURLWithPath: "/tmp")),
              environment["NOLA_TEST_MODE"] == "1", environment["NOLA_USE_REAL_KEYCHAIN_IN_TESTS"] == "0" else {
            throw MacApplicationPortError.invalidResponse
        }
        let configurations = MacAppleChatConfigurationStore.shared
        try configurations.replaceEndpoints([MacRemoteEndpointConfiguration(name: "Mention acceptance",
            baseURL: config.endpoint, apiKey: config.apiKey,
            models: [MacRemoteModelConfiguration(modelName: config.model, executionCapabilities: .init(toolCalls: true))],
            wireProtocol: .chatCompletions)])
        let port = try MacApplicationPort(databaseURL: database)
        let repository = MacCanonicalConversationRepository(store: port)
        let conversation = "mac-mention-real-\(UUID().uuidString)"
        let model = MacWorkspaceModel(initialConversationID: conversation, canonicalRepository: repository,
            localAgentRuntime: MacAgentHostRuntime.shared, remoteHost: PersistenceTestRemoteHost(), modelApplication: port)
        model.executionLocation = .thisMac
        model.newConversation()
        let createdConversation = try XCTUnwrap(model.selectedConversationID)
        let service = MacAgentRoomService(runtime: port)
        var frozenTemplate = PromptTemplate(name: "Frozen arithmetic definition", language: "en",
            content: "You are the original arithmetic expert. Answer with the exact result marker requested by the user. Do not use tools.",
            description: "A real definition snapshot for edited resend acceptance")
        try MacPromptLibrary.service.addTemplate(frozenTemplate)
        guard case .object(var expertDefaults) = try repository.agentModelDefaults() else { throw MacApplicationPortError.invalidResponse }
        expertDefaults["selectedPromptTemplateID"] = .string(frozenTemplate.id.uuidString)
        let directory = try service.createAgent(name: "MacArithmetic", description: "Arithmetic acceptance expert",
            defaults: .object(expertDefaults), directory: service.directory())
        let expert = try XCTUnwrap(directory.agents.first { $0.name == "MacArithmetic" })
        let root = MacChatWorkspaceView(initialConversationID: createdConversation, forcesInspectorVisible: false,
            canonicalRepository: repository, localAgentRuntime: MacAgentHostRuntime.shared,
            remoteHost: PersistenceTestRemoteHost(),
            inspectorDependencies: MacInspectorDependencies(dataRegistry: MacDataSourceRegistry.shared, platformData: MacPlatformDataService.shared,
                webSources: MacWebSourceStore.shared, chromeMessaging: MacChromeMessagingService.shared),
            model: model).frame(width: 1100, height: 800)
        let hosting = NSHostingView(rootView: root)
        hosting.frame = NSRect(x: 0, y: 0, width: 1100, height: 800)
        let window = NSWindow(contentRect: hosting.bounds, styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer { model.presentationDidDetach(); window.orderOut(nil); window.contentView = nil }
        await settle(hosting)
        await model.loadSelectedConversationIfNeeded()
        func editor(_ view: NSView) -> MacAttachmentPasteTextView? {
            if let input = view as? MacAttachmentPasteTextView { return input }
            return view.subviews.lazy.compactMap { editor($0) }.first
        }
        let input = try XCTUnwrap(editor(hosting))
        XCTAssertTrue(window.makeFirstResponder(input))
        input.insertText("@MacArith", replacementRange: NSRange(location: NSNotFound, length: 0))
        await settle(hosting)
        XCTAssertEqual(model.draft, "@MacArith", "Native input must update the actual conversation draft")
        let mentionID = "chat.mention." + expert.id
        let deadline = ContinuousClock.now + .seconds(15)
        while accessibilityElement(hosting, identifier: mentionID) == nil && ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        let candidate = try XCTUnwrap(accessibilityElement(hosting, identifier: mentionID), accessibilityTree(hosting))
        XCTAssertEqual(candidate.accessibilityPerformPress?(), true)
        await settle(hosting)
        XCTAssertEqual(model.draft, "")
        XCTAssertTrue(window.makeFirstResponder(input))
        input.insertText("Compute 37 times 19. Reply MAC_EXPERT_RESULT= followed by the result. Do not use tools.",
            replacementRange: NSRange(location: NSNotFound, length: 0))
        await settle(hosting)
        let send = try XCTUnwrap(accessibilityElement(hosting, identifier: "mac-composer-send"))
        XCTAssertEqual(send.accessibilityPerformPress?(), true)
        let answerDeadline = ContinuousClock.now + .seconds(180)
        while !accessibilityTree(hosting).contains("MAC_EXPERT_RESULT=703") && ContinuousClock.now < answerDeadline {
            for id in ["chat.expert-execution-error", "chat.expert-admission-error"] {
                if accessibilityElement(hosting, identifier: id) != nil {
                    XCTFail(accessibilityTree(hosting)); throw MacApplicationPortError.invalidResponse
                }
            }
            try await Task.sleep(for: .milliseconds(200))
        }
        let association = try XCTUnwrap(service.forConversation(createdConversation))
        let participant = try XCTUnwrap(association.room.room.participants.first { $0.agentID == expert.id })
        let receipt = try XCTUnwrap(service.submissions(roomID: association.roomID).first {
            $0.selectedParticipantID == participant.id && $0.input.conversationID == createdConversation
        })
        let childRunID = try XCTUnwrap(receipt.executionID)
        while try port.canonicalRunTerminal(conversationID: participant.contextConversationID, runID: childRunID) == nil,
              ContinuousClock.now < answerDeadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        let terminal = try XCTUnwrap(try port.canonicalRunTerminal(conversationID: participant.contextConversationID, runID: childRunID))
        let receiptDeadline = ContinuousClock.now + .seconds(10)
        while try service.submission(roomID: association.roomID, submissionID: receipt.id)?.state == "executing",
              ContinuousClock.now < receiptDeadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        await settle(hosting)
        let text = accessibilityTree(hosting)
        XCTAssertFalse(text.contains("mac-experts-resume-"), "A committed expert answer must not retain a cold-recovery action")
        let loadedChild = try await repository.loadAsync(conversationID: participant.contextConversationID)
        let child = try XCTUnwrap(loadedChild)
        try persistCanonicalTranscript(child.messages, to: artifactURL(environment, "NOLA_MAC_REAL_UI_TRANSCRIPT"))
        // This short expert task records one observed terminal UI state. The
        // separate long-response case measures incremental stream transitions.
        try JSONSerialization.data(withJSONObject: [["stage": "expert-final", "running": false,
            "runID": terminal.runID, "assistantCharacters": child.messages.last(where: { $0.role == .assistant })?.text.count ?? 0]])
            .write(to: artifactURL(environment, "NOLA_MAC_REAL_UI_TIMELINE"))
        try text.write(to: artifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY"), atomically: true, encoding: .utf8)
        try capturePNG(hosting, to: artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT"))
        XCTAssertTrue(text.contains("MAC_EXPERT_RESULT=703"), text)

        // Change the live directory and template after the source Run. The
        // production edit action must use the frozen original definition.
        let sourceParentID = try XCTUnwrap(receipt.input.parentMessageID)
        let sourceManifest = try XCTUnwrap(service.expertMessageSource(conversationID: createdConversation, messageID: sourceParentID))
        let sourceEntry = try XCTUnwrap(sourceManifest.entries.first)
        let sourceDefinition = try XCTUnwrap(sourceEntry.instruction_witness?.prompt_definitions.first {
            $0.component_id == "prompt_template:" + frozenTemplate.id.uuidString
        })
        XCTAssertEqual(sourceDefinition.content, frozenTemplate.content)
        let originalTemplateBody = frozenTemplate.content
        frozenTemplate.content = "This is the changed current template. Reply CURRENT_TEMPLATE_ONLY."
        try MacPromptLibrary.service.updateTemplate(frozenTemplate)
        XCTAssertEqual(MacPromptLibrary.service.readableTemplate(id: frozenTemplate.id)?.content, frozenTemplate.content)
        _ = try service.updateAgent(expert, name: "Changed current directory name", description: "Changed after the source Run",
            availability: "archived", directory: service.directory())
        let sourceUser = try XCTUnwrap(model.selectedMessages.first { $0.role == .user })
        let expertDraft = "UNSENT_EXPERT_SOURCE_DRAFT"
        try await typeRealComposer(expertDraft, in: hosting, window: window)
        let expertTarget = try await editVisibleUserMessage(sourceUser, source: createdConversation,
            text: "Compute 41 times 17. Reply MAC_EXPERT_EDIT_RESULT= followed by the result. Do not use tools.",
            model: model, in: hosting, window: window)
        let expertReplyDeadline = ContinuousClock.now + .seconds(180)
        while !accessibilityTree(hosting).contains("MAC_EXPERT_EDIT_RESULT=697") && ContinuousClock.now < expertReplyDeadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        let prepared = try XCTUnwrap(port.loadPreparedExpertResend(expertTarget))
        XCTAssertNotEqual(prepared.room_id, association.roomID)
        XCTAssertEqual(prepared.entries.count, sourceManifest.entries.count)
        let editedEntry = try XCTUnwrap(prepared.entries.first)
        XCTAssertNotEqual(editedEntry.participant.context_conversation_id, participant.contextConversationID)
        XCTAssertNotEqual(editedEntry.prepared.run_id, childRunID)
        let freshExpertParent = try await repository.loadCanonicalConversationAsync(conversationID: expertTarget)
        XCTAssertEqual(freshExpertParent?.messages.filter { $0.role == "user" }.count, 1)
        XCTAssertEqual(editedEntry.original.display_name, "MacArithmetic")
        XCTAssertEqual(editedEntry.original.participant, sourceEntry.participant)
        XCTAssertEqual(editedEntry.original.defaults, sourceEntry.defaults)
        XCTAssertEqual(editedEntry.prepared.source.model.primary, sourceEntry.model_witness?.model.primary)
        XCTAssertEqual(editedEntry.prepared.source.model.generation, sourceEntry.model_witness?.model.generation)
        XCTAssertEqual(editedEntry.prepared.source.model.model_capability, sourceEntry.model_witness?.model.model_capability)
        XCTAssertEqual(editedEntry.original.instruction_witness?.prompt_definitions.first {
            $0.component_id == sourceDefinition.component_id
        }?.content, originalTemplateBody)
        let editedChild = try await waitForRealTerminal(conversation: editedEntry.participant.context_conversation_id,
            run: editedEntry.prepared.run_id, repository: repository, application: port)
        XCTAssertTrue(editedChild.messages.contains { $0.role == .assistant && $0.text.contains("MAC_EXPERT_EDIT_RESULT=697") })
        let actualExpertManifest = try XCTUnwrap(service.expertMessageSource(conversationID: expertTarget, messageID: prepared.parent_message_id))
        let actualExpertEntry = try XCTUnwrap(actualExpertManifest.entries.first)
        XCTAssertEqual(actualExpertEntry.model_witness?.model.primary, sourceEntry.model_witness?.model.primary)
        XCTAssertEqual(actualExpertEntry.model_witness?.model.generation, sourceEntry.model_witness?.model.generation)
        XCTAssertEqual(actualExpertEntry.instruction_witness?.prompt_definitions.first {
            $0.component_id == sourceDefinition.component_id
        }?.content, originalTemplateBody)
        XCTAssertEqual(try service.forConversation(expertTarget)?.room.room.participants.first?.displayName, "MacArithmetic")
        try capturePNG(hosting, to: artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT").deletingLastPathComponent().appendingPathComponent("expert-edited-ui.png"))
        try await selectRealConversation(createdConversation, model: model, in: hosting)
        XCTAssertEqual(model.draft, expertDraft)
        XCTAssertEqual(model.selectedMessages.first(where: { $0.role == .user })?.text, sourceUser.text)
        XCTAssertEqual(try service.expertMessageSource(conversationID: createdConversation, messageID: sourceParentID)?.input.text,
                       sourceManifest.input.text)

        // The same production editor must also work for an ordinary message.
        let newConversation = try XCTUnwrap(accessibilityElement(hosting, identifier: "workspace.newConversation"))
        XCTAssertEqual(newConversation.accessibilityPerformPress?(), true)
        await settle(hosting)
        let ordinarySource = try XCTUnwrap(model.selectedConversationID)
        XCTAssertNotEqual(ordinarySource, createdConversation)
        try await typeRealComposer("Compute 31 times 29. Reply MAC_ORDINARY_RESULT= followed by the result. Do not use tools.", in: hosting, window: window)
        let ordinarySend = try XCTUnwrap(accessibilityElement(hosting, identifier: "mac-composer-send"))
        XCTAssertEqual(ordinarySend.accessibilityPerformPress?(), true)
        let ordinaryDeadline = ContinuousClock.now + .seconds(180)
        while (model.isRunning || !accessibilityTree(hosting).contains("MAC_ORDINARY_RESULT=899")) && ContinuousClock.now < ordinaryDeadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertFalse(model.isRunning)
        let ordinaryUser = try XCTUnwrap(model.selectedMessages.first { $0.role == .user })
        let ordinaryOriginal = model.selectedMessages
        let ordinaryDraft = "UNSENT_ORDINARY_SOURCE_DRAFT"
        try await typeRealComposer(ordinaryDraft, in: hosting, window: window)
        let ordinaryTarget = try await editVisibleUserMessage(ordinaryUser, source: ordinarySource,
            text: "Compute 47 times 13. Reply MAC_ORDINARY_EDIT_RESULT= followed by the result. Do not use tools.",
            model: model, in: hosting, window: window)
        let ordinaryPrepared = try XCTUnwrap(port.loadPreparedEditedResend(ordinaryTarget))
        let ordinaryEdited = try await waitForRealTerminal(conversation: ordinaryTarget, run: ordinaryPrepared.run_id,
            repository: repository, application: port)
        XCTAssertTrue(ordinaryEdited.messages.contains { $0.role == .assistant && $0.text.contains("MAC_ORDINARY_EDIT_RESULT=611") })
        XCTAssertEqual(ordinaryPrepared.admitted_run_id, ordinaryPrepared.run_id)
        let actualOrdinary = try XCTUnwrap(port.loadExactRunInvocation(conversationID: ordinaryTarget,
            expectedRunID: ordinaryPrepared.run_id, assistantMessageID: ordinaryPrepared.assistant_message_id))
        guard case .object(let ordinaryBinding) = actualOrdinary.hostInvocation else { throw MacApplicationPortError.invalidResponse }
        XCTAssertEqual(ordinaryBinding["primary"], ordinaryPrepared.source.model.primary)
        XCTAssertEqual(ordinaryBinding["generation"], ordinaryPrepared.source.model.generation)
        XCTAssertNotEqual(ordinaryPrepared.run_id, ordinaryPrepared.source.run_id)
        XCTAssertEqual(ordinaryEdited.messages.filter { $0.role == .user }.count, 1)
        XCTAssertNil(try service.forConversation(ordinaryTarget))
        try await selectRealConversation(ordinarySource, model: model, in: hosting)
        XCTAssertEqual(model.draft, ordinaryDraft)
        XCTAssertEqual(model.selectedMessages.first(where: { $0.role == .user })?.text, ordinaryUser.text)
        try await selectRealConversation(ordinaryTarget, model: model, in: hosting)
        await settle(hosting)
        let allMessages = child.messages + editedChild.messages + ordinaryOriginal + ordinaryEdited.messages
        try persistCanonicalTranscript(allMessages, to: artifactURL(environment, "NOLA_MAC_REAL_UI_TRANSCRIPT"))
        try JSONSerialization.data(withJSONObject: [
            ["stage":"expert-source", "running":false, "assistantCharacters":child.messages.last?.text.count ?? 0],
            ["stage":"expert-edited", "running":false, "assistantCharacters":editedChild.messages.last?.text.count ?? 0],
            ["stage":"ordinary-source", "running":false, "assistantCharacters":ordinaryOriginal.last?.text.count ?? 0],
            ["stage":"ordinary-edited", "running":false, "assistantCharacters":ordinaryEdited.messages.last?.text.count ?? 0]
        ]).write(to: artifactURL(environment, "NOLA_MAC_REAL_UI_TIMELINE"))
        try accessibilityTree(hosting).write(to: artifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY"), atomically: true, encoding: .utf8)
        try capturePNG(hosting, to: artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT"))
    }

    private func typeRealComposer(_ text: String, in root: NSView, window: NSWindow) async throws {
        func find(_ view: NSView) -> MacAttachmentPasteTextView? {
            (view as? MacAttachmentPasteTextView) ?? view.subviews.lazy.compactMap { find($0) }.first
        }
        let input = try XCTUnwrap(find(root))
        XCTAssertTrue(window.makeFirstResponder(input))
        input.insertText(text, replacementRange: NSRange(location: 0, length: input.string.utf16.count))
        window.makeFirstResponder(root)
        await settle(root)
    }

    private func editVisibleUserMessage(_ message: MacChatMessage, source: String, text: String,
                                        model: MacWorkspaceModel, in root: NSView, window: NSWindow) async throws -> String {
        let id = "mac-message-edit-" + message.id.uuidString
        let deadline = ContinuousClock.now + .seconds(45)
        while accessibilityElement(root, identifier: id)?.isAccessibilityEnabled?() != true && ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        let edit = try XCTUnwrap(accessibilityElement(root, identifier: id), accessibilityTree(root))
        XCTAssertEqual(edit.accessibilityPerformPress?(), true)
        while window.attachedSheet == nil && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(50)) }
        let sheet = try XCTUnwrap(window.attachedSheet, accessibilityTree(root))
        let content = try XCTUnwrap(sheet.contentView)
        await settle(content)
        let field = try XCTUnwrap(accessibilityElement(content, identifier: "mac-message-edit-text"), accessibilityTree(content))
        let frame = sheet.convertFromScreen(field.accessibilityFrame?() ?? .zero)
        for kind in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let click = try XCTUnwrap(NSEvent.mouseEvent(with: kind, location: NSPoint(x: frame.midX, y: frame.midY),
                modifierFlags: [], timestamp: 0, windowNumber: sheet.windowNumber, context: nil,
                eventNumber: 1, clickCount: 1, pressure: kind == .leftMouseDown ? 1 : 0))
            sheet.sendEvent(click)
        }
        await settle(content)
        let editor = try XCTUnwrap(sheet.firstResponder as? NSTextInputClient, accessibilityTree(content))
        editor.insertText(text, replacementRange: NSRange(location: 0, length: message.text.utf16.count))
        sheet.makeFirstResponder(content)
        await settle(content)
        let submit = try XCTUnwrap(accessibilityElement(content, identifier: "mac-message-edit-submit"), accessibilityTree(content))
        XCTAssertEqual(submit.isAccessibilityEnabled?(), true)
        XCTAssertEqual(submit.accessibilityPerformPress?(), true)
        while (model.selectedConversationID == source || model.selectedConversationID == nil) && ContinuousClock.now < deadline {
            if accessibilityElement(content, identifier: "mac-message-edit-error") != nil {
                XCTFail(accessibilityTree(content)); throw MacApplicationPortError.invalidResponse
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        let target = try XCTUnwrap(model.selectedConversationID, accessibilityTree(content))
        XCTAssertNotEqual(target, source, accessibilityTree(content))
        guard target != source else { throw MacApplicationPortError.invalidResponse }
        while window.attachedSheet != nil && ContinuousClock.now < deadline {
            if accessibilityElement(content, identifier: "mac-message-edit-error") != nil {
                XCTFail(accessibilityTree(content)); throw MacApplicationPortError.invalidResponse
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertNil(window.attachedSheet, accessibilityTree(content))
        guard window.attachedSheet == nil else { throw MacApplicationPortError.invalidResponse }
        await model.loadSelectedConversationIfNeeded()
        await settle(root)
        return target
    }

    private func selectRealConversation(_ id: String, model: MacWorkspaceModel, in root: NSView) async throws {
        let row = try XCTUnwrap(accessibilityElement(root, identifier: "workspace.conversation." + id), accessibilityTree(root))
        XCTAssertEqual(row.accessibilityPerformPress?(), true)
        // Let the production selection Task own its window load. Starting a
        // second select here would cancel its page generation and can observe
        // the intentionally empty presentation between the two loads.
        let deadline = ContinuousClock.now + .seconds(30)
        while (model.selectedConversationID != id || model.loadingConversationID == id || model.selectedMessages.isEmpty)
                && ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(50))
        }
        await settle(root)
        XCTAssertEqual(model.selectedConversationID, id)
        XCTAssertFalse(model.selectedMessages.isEmpty,
            "selected=\(model.selectedConversationID ?? "nil") attached=\(model.isPresentationAttached) persisted=\(model.selectedConversation?.isPersisted == true) loading=\(model.loadingConversationID ?? "nil") stored=\(model.messagesByConversation[id]?.count ?? -1) running=\(model.isRunning)\n" + accessibilityTree(root))
    }

    private func waitForRealTerminal(conversation: String, run: String, repository: MacCanonicalConversationRepository,
                                     application: MacApplicationPort) async throws -> MacStoredConversation {
        let deadline = ContinuousClock.now + .seconds(180)
        while try application.canonicalRunTerminal(conversationID: conversation, runID: run) == nil,
              ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(100)) }
        XCTAssertNotNil(try application.canonicalRunTerminal(conversationID: conversation, runID: run))
        let loaded = try await repository.loadAsync(conversationID: conversation)
        return try XCTUnwrap(loaded)
    }

    func testRealA2UICancellationAndWindowReturnPreserveCommittedSurface() async throws {
        let priorActivationPolicy = NSApp.activationPolicy()
        defer { NSApp.setActivationPolicy(priorActivationPolicy) }
        NSApp.setActivationPolicy(.regular)
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1", let config = realLLMConfiguration(environment) else {
            throw XCTSkip("Requires isolated real-provider XCTest runner")
        }
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        print("A2UI acceptance stage: resolve isolated database")
        let database = MacApplicationPort.defaultDatabaseURL
        guard MacAppEnvironment.contains(database, in: URL(fileURLWithPath: "/tmp")),
              environment["NOLA_TEST_MODE"] == "1", environment["NOLA_USE_REAL_KEYCHAIN_IN_TESTS"] == "0" else {
            throw MacApplicationPortError.invalidResponse
        }
        let wire = try XCTUnwrap(MacModelWireProtocol(rawValue: try XCTUnwrap(environment["NOLA_REAL_LLM_WIRE_PROTOCOL"])))
        let configurations = MacAppleChatConfigurationStore.shared
        let prior = configurations.endpoints()
        print("A2UI acceptance stage: persist model configuration")
        try configurations.replaceEndpoints([MacRemoteEndpointConfiguration(name: "A2UI isolated acceptance",
            baseURL: config.endpoint, apiKey: config.apiKey,
            models: [MacRemoteModelConfiguration(modelName: config.model, executionCapabilities: .init(toolCalls: true))],
            wireProtocol: wire)])
        defer { try? configurations.replaceEndpoints(prior) }
        print("A2UI acceptance stage: initialize canonical port")
        let port = try MacApplicationPort(databaseURL: database)
        let repository = MacCanonicalConversationRepository(store: port)
        let conversation = "mac-a2ui-cancel-\(UUID().uuidString)"
        print("A2UI acceptance stage: initialize workspace")
        let model = MacWorkspaceModel(initialConversationID: conversation, canonicalRepository: repository,
            localAgentRuntime: MacAgentHostRuntime.shared, remoteHost: PersistenceTestRemoteHost(), modelApplication: port)
        model.executionLocation = .thisMac
        model.selectedConversationID = conversation
        let root = MacChatWorkspaceView(initialConversationID: conversation, forcesInspectorVisible: false,
            canonicalRepository: repository, localAgentRuntime: MacAgentHostRuntime.shared,
            remoteHost: PersistenceTestRemoteHost(),
            inspectorDependencies: MacInspectorDependencies(dataRegistry: MacDataSourceRegistry.shared, platformData: MacPlatformDataService.shared,
                webSources: MacWebSourceStore.shared, chromeMessaging: MacChromeMessagingService.shared),
            model: model).frame(width: 1100, height: 800)
        print("A2UI acceptance stage: present workspace UI")
        let hosting = NSHostingView(rootView: root)
        hosting.frame = NSRect(x: 0, y: 0, width: 1100, height: 800)
        let window = NSWindow(contentRect: hosting.bounds, styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
        window.contentView = hosting
        window.orderFrontRegardless()
        defer { window.orderOut(nil); window.contentView = nil }
        await settle(hosting)
        // Verify the owned test-host window can complete AppKit transitions
        // before spending any provider request on this scenario.
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
        window.miniaturize(nil)
        let windowDeadline = ContinuousClock.now + .seconds(5)
        while !window.isMiniaturized && ContinuousClock.now < windowDeadline {
            try await Task.sleep(for: .milliseconds(50))
        }
        guard window.isMiniaturized else {
            XCTFail("The isolated test-host window could not be minimized")
            throw MacApplicationPortError.invalidResponse
        }
        window.deminiaturize(nil)
        window.makeKeyAndOrderFront(nil)
        await settle(hosting)
        defer { if model.isRunning { model.stop(reason: "isolated cancellation test cleanup") } }
        var timeline: [[String: Any]] = []
        var approvals: Set<String> = []
        let screenshot = try artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT")
        let directory = screenshot.deletingLastPathComponent()
        try await typeRealComposer("First discover render_ui using tool_search. You must call the actual render_ui tool and verify its committed receipt; a text description is not a published surface. Publish one A2UI 0.9.1 surfaceId cancellation-plan with Text title Safe observatory plan, Text body describing three real operational tradeoffs for an amateur telescope observation night (weather, equipment, timing), and Text status PLAN_READY. Keep the body under 180 words. Finish normally after publishing. Do not use any other tools.", in: hosting, window: window)
        let send = try XCTUnwrap(accessibilityElement(hosting, identifier: "mac-composer-send"))
        XCTAssertEqual(send.accessibilityPerformPress?(), true)
        try await observeRun(stage: "a2ui-cancel-baseline", model: model, hosting: hosting, timeline: &timeline,
            permissionRequestIDs: &approvals, timeout: .seconds(240),
            timelineURL: try artifactURL(environment, "NOLA_MAC_REAL_UI_TIMELINE"), screenshotURL: screenshot,
            accessibilityURL: try artifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY"), requireIncrementalText: false)
        let initial = try XCTUnwrap(port.listUISurfaces(conversationID: conversation).first { $0.phase == .ready })
        try await typeRealComposer("Update the SAME cancellation-plan surface using FOUR separate render_ui calls. In the first call analyze weather and visibility risks, second compare telescope equipment choices, third create a precise observation timeline, fourth summarize practical go/no-go criteria. Each body should contain 100-150 words of distinct useful analysis and status PLAN_REVIEW_1 through PLAN_REVIEW_4. Preserve the identity and current revision. Do not use other tools or execute anything.", in: hosting, window: window)
        XCTAssertEqual(accessibilityElement(hosting, identifier: "mac-composer-send")?.accessibilityPerformPress?(), true)
        let deadline = ContinuousClock.now + .seconds(240)
        var minimized = false
        var activeRun: String?
        while ContinuousClock.now < deadline {
            if let active = try port.loadConversationWindow(conversation).projection.activeRun { activeRun = active.runID }
            if let permission = model.pendingRemotePermission {
                guard permission.toolName == "render_ui" else {
                    model.resolvePermission(allow: false, scope: .once)
                    throw MacApplicationPortError.invalidResponse
                }
                if !approvals.contains(permission.id), let allow = accessibilityElement(hosting, identifier: "chat.permission.allow-once") {
                    XCTAssertEqual(allow.accessibilityPerformPress?(), true)
                    approvals.insert(permission.id)
                }
            }
            if model.isRunning && !minimized {
                NSApp.activate(ignoringOtherApps: true)
                window.makeKeyAndOrderFront(nil)
                window.miniaturize(nil)
                let minimizedDeadline = ContinuousClock.now + .seconds(5)
                while !window.isMiniaturized && ContinuousClock.now < minimizedDeadline {
                    try await Task.sleep(for: .milliseconds(50))
                }
                XCTAssertTrue(window.isMiniaturized, "Wait for the AppKit window transition to finish")
                window.deminiaturize(nil)
                window.makeKeyAndOrderFront(nil)
                minimized = true
                await settle(hosting)
                try capturePNG(hosting, to: directory.appendingPathComponent("a2ui-cancel-window-return.png"))
            }
            if model.isRunning, let current = try port.listUISurfaces(conversationID: conversation).first(where: { $0.identity == initial.identity }), current.revision > initial.revision {
                try capturePNG(hosting, to: directory.appendingPathComponent("a2ui-cancel-before-stop.png"))
                let stop = try XCTUnwrap(accessibilityElement(hosting, identifier: "mac-composer-stop"))
                XCTAssertEqual(stop.accessibilityPerformPress?(), true)
                break
            }
            if activeRun != nil && !model.isRunning { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertTrue(minimized)
        let run = try XCTUnwrap(activeRun)
        let terminalDeadline = ContinuousClock.now + .seconds(30)
        while try port.canonicalRunTerminal(conversationID: conversation, runID: run) == nil, ContinuousClock.now < terminalDeadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        let terminal = try XCTUnwrap(port.canonicalRunTerminal(conversationID: conversation, runID: run))
        XCTAssertEqual(terminal.result.value.kind, .cancelled)
        XCTAssertNil(try port.loadConversationWindow(conversation).projection.activeRun)
        let committed = try XCTUnwrap(port.listUISurfaces(conversationID: conversation).first { $0.identity == initial.identity })
        XCTAssertGreaterThan(committed.revision, initial.revision)
        model.newConversation()
        model.selectedConversationID = conversation
        await model.loadSelectedConversationIfNeeded()
        await settle(hosting)
        let cold = try MacApplicationPort(databaseURL: database)
        XCTAssertEqual(try cold.listUISurfaces(conversationID: conversation).first { $0.identity == initial.identity }?.revision, committed.revision)
        try capturePNG(hosting, to: screenshot)
        try accessibilityTree(hosting).write(to: artifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY"), atomically: true, encoding: .utf8)
        try persistCanonicalTranscript(model.selectedMessages, to: artifactURL(environment, "NOLA_MAC_REAL_UI_TRANSCRIPT"))
        try persistTimeline([["stage":"a2ui-cancel-window-return", "running":false, "resultKind":"cancelled", "revision":committed.revision, "windowMinimizedAndRestored":minimized]], to: artifactURL(environment, "NOLA_MAC_REAL_UI_TIMELINE"))
    }

    func testRealA2UISurfacePublishesRendersAndRestoresExactHistory() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1", let config = realLLMConfiguration(environment) else {
            throw XCTSkip("Requires isolated real-provider XCTest runner")
        }
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        print("A2UI acceptance stage: resolve isolated database")
        let database = MacApplicationPort.defaultDatabaseURL
        guard MacAppEnvironment.contains(database, in: URL(fileURLWithPath: "/tmp")),
              environment["NOLA_TEST_MODE"] == "1", environment["NOLA_USE_REAL_KEYCHAIN_IN_TESTS"] == "0" else {
            throw MacApplicationPortError.invalidResponse
        }
        let wire = try XCTUnwrap(MacModelWireProtocol(rawValue: try XCTUnwrap(environment["NOLA_REAL_LLM_WIRE_PROTOCOL"])))
        let configurations = MacAppleChatConfigurationStore.shared
        let prior = configurations.endpoints()
        print("A2UI acceptance stage: persist model configuration")
        try configurations.replaceEndpoints([MacRemoteEndpointConfiguration(name: "A2UI isolated acceptance",
            baseURL: config.endpoint, apiKey: config.apiKey,
            models: [MacRemoteModelConfiguration(modelName: config.model, executionCapabilities: .init(toolCalls: true))],
            wireProtocol: wire)])
        defer { try? configurations.replaceEndpoints(prior) }
        print("A2UI acceptance stage: initialize canonical port")
        let port = try MacApplicationPort(databaseURL: database)
        let repository = MacCanonicalConversationRepository(store: port)
        let conversation = "mac-a2ui-real-\(UUID().uuidString)"
        print("A2UI acceptance stage: initialize workspace")
        let model = MacWorkspaceModel(initialConversationID: conversation, canonicalRepository: repository,
            localAgentRuntime: MacAgentHostRuntime.shared, remoteHost: PersistenceTestRemoteHost(), modelApplication: port)
        model.executionLocation = .thisMac
        model.selectedConversationID = conversation
        let root = MacChatWorkspaceView(initialConversationID: conversation, forcesInspectorVisible: false,
            canonicalRepository: repository, localAgentRuntime: MacAgentHostRuntime.shared,
            remoteHost: PersistenceTestRemoteHost(),
            inspectorDependencies: MacInspectorDependencies(dataRegistry: MacDataSourceRegistry.shared, platformData: MacPlatformDataService.shared,
                webSources: MacWebSourceStore.shared, chromeMessaging: MacChromeMessagingService.shared),
            model: model).frame(width: 1100, height: 800)
        print("A2UI acceptance stage: present workspace UI")
        let hosting = NSHostingView(rootView: root)
        hosting.frame = NSRect(x: 0, y: 0, width: 1100, height: 800)
        let window = NSWindow(contentRect: hosting.bounds, styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.contentView = hosting
        window.orderFrontRegardless()
        defer { window.orderOut(nil); window.contentView = nil }
        await settle(hosting)
        var timeline: [[String: Any]] = []
        var approvals: Set<String> = []
        model.draft = "Use render_ui to publish an A2UI 0.9.1 form surfaceId mac-acceptance, Text title NOLA-A2UI-REAL, TextField id name labeled Acceptance name bound to /name initially empty, Button id submit labeled Submit acceptance with submit event and context name bound to /name, Text id status Awaiting input. When a real UI action arrives, read its submitted name and update this SAME surface status to ACK followed by that name, preserving the field and button. Do not invent a value. Publish now."
        print("A2UI acceptance stage: send real prompt")
        model.send()
        try await observeRun(stage: "a2ui", model: model, hosting: hosting, timeline: &timeline,
            permissionRequestIDs: &approvals, timeout: .seconds(240),
            timelineURL: try artifactURL(environment, "NOLA_MAC_REAL_UI_TIMELINE"),
            screenshotURL: try artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT"),
            accessibilityURL: try artifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY"), requireIncrementalText: false)
        let surfaces = try port.listUISurfaces(conversationID: conversation)
        let surface = try XCTUnwrap(surfaces.first(where: { $0.phase == .ready }))
        XCTAssertEqual(surface.identity.owner.conversation_id, conversation)
        XCTAssertTrue(surface.nodes.values.contains(where: { $0.component == "TextField" }))
        XCTAssertTrue(surface.nodes.values.contains(where: { $0.component == "Button" && !$0.actions.isEmpty }))
        await settle(hosting)
        let rendered = accessibilityTree(hosting)
        XCTAssertTrue(rendered.contains("NOLA-A2UI-REAL"), "The Core Surface must be visible in the product UI")
        try rendered.write(to: try artifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY"), atomically: true, encoding: .utf8)
        try capturePNG(hosting, to: try artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT"))
        // Edit the actual AppKit field editor hosted by SwiftUI, then press
        // the real generated Button through its accessibility action.
        func textField(in view: NSView) -> NSTextField? {
            if let field = view as? NSTextField, field.placeholderString == "Acceptance name" { return field }
            return view.subviews.lazy.compactMap { textField(in: $0) }.first
        }
        let field = try XCTUnwrap(textField(in: hosting))
        window.makeKeyAndOrderFront(nil)
        field.selectText(nil)
        let editor = try XCTUnwrap(field.currentEditor() as? NSTextView)
        let token = "ORCHID" + UUID().uuidString.replacingOccurrences(of: "-", with: "").prefix(8)
        editor.insertText(token, replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
        window.makeFirstResponder(hosting)
        await settle(hosting)
        XCTAssertEqual(field.stringValue, token)
        let evidenceDirectory = try artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT").deletingLastPathComponent()
        try capturePNG(hosting, to: evidenceDirectory.appendingPathComponent("a2ui-edited-before-submit.png"))
        try accessibilityTree(hosting).write(to: evidenceDirectory.appendingPathComponent("a2ui-edited-before-submit.txt"), atomically: true, encoding: .utf8)
        // Core previews edits asynchronously; wait for the actual control to
        // become enabled, not a fixed settle delay or a stale AX element.
        let validationDeadline = ContinuousClock.now + .seconds(10)
        while accessibilityElement(hosting, identifier: "a2ui.action.submit")?.isAccessibilityEnabled?() != true,
              ContinuousClock.now < validationDeadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        let submit = try XCTUnwrap(accessibilityElement(hosting, identifier: "a2ui.action.submit"))
        XCTAssertEqual(submit.isAccessibilityEnabled?(), true)
        let submitFrame = try XCTUnwrap(submit.accessibilityFrame?())
        XCTAssertFalse(submitFrame.isEmpty)
        let localFrame = window.convertFromScreen(submitFrame)
        let point = NSPoint(x: localFrame.midX, y: localFrame.midY)
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            window.sendEvent(try XCTUnwrap(NSEvent.mouseEvent(with: type, location: point,
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, eventNumber: 0,
                clickCount: 1, pressure: 1)))
        }
        let admissionDeadline = ContinuousClock.now + .seconds(20)
        while !model.isRunning && ContinuousClock.now < admissionDeadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        guard model.isRunning else {
            try capturePNG(hosting, to: evidenceDirectory.appendingPathComponent("a2ui-submit-not-admitted.png"))
            XCTFail("A committed UI action must start its owning conversation's next Run; preserve this isolated store for receipt reconciliation")
            throw MacApplicationPortError.invalidResponse
        }
        try await observeRun(stage: "a2ui-submitted", model: model, hosting: hosting, timeline: &timeline,
            permissionRequestIDs: &approvals, timeout: .seconds(240),
            timelineURL: try artifactURL(environment, "NOLA_MAC_REAL_UI_TIMELINE"),
            screenshotURL: try artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT"),
            accessibilityURL: try artifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY"), requireIncrementalText: false)
        guard try port.loadConversationWindow(conversation).projection.activeRun == nil else {
            XCTFail("The UI action's owning Core Run must reach a real terminal state; a stopped UI observer is insufficient")
            throw MacApplicationPortError.invalidResponse
        }
        let updated = try XCTUnwrap(port.listUISurfaces(conversationID: conversation).first { $0.identity == surface.identity })
        XCTAssertGreaterThan(updated.revision, surface.revision)
        XCTAssertTrue(updated.nodes.values.contains { $0.props["text"]?.journalString == "ACK " + token })
        XCTAssertEqual(try port.listUISurfaces(conversationID: conversation).filter { $0.phase == .ready }.count, 1)
        model.newConversation()
        XCTAssertNotEqual(model.selectedConversationID, conversation)
        await settle(hosting)
        XCTAssertNil(accessibilityElement(hosting, identifier: "a2ui.field.name"))
        model.selectedConversationID = conversation
        await settle(hosting)
        XCTAssertTrue(accessibilityTree(hosting).contains("ACK " + token))
        let cold = try MacApplicationPort(databaseURL: database)
        let history = try XCTUnwrap(cold.readUISurfaceRevision(identity: surface.identity, revision: surface.revision))
        XCTAssertEqual(history.revision, surface.revision)
        XCTAssertEqual(history.phase, .readOnly)
        XCTAssertTrue(history.nodes.values.allSatisfy { $0.actions.isEmpty && $0.bindings.isEmpty })
        let snapshot = try cold.readUISurfaceChanges(identity: surface.identity, baseRevision: nil)
            .applying(to: nil, identity: surface.identity)
        XCTAssertEqual(snapshot?.identity, surface.identity)
        XCTAssertEqual(snapshot?.revision, updated.revision)
        XCTAssertTrue(snapshot?.nodes.values.contains { $0.props["text"]?.journalString == "ACK " + token } == true)
        let transcript = model.selectedMessages
        try persistCanonicalTranscript(transcript, to: try artifactURL(environment, "NOLA_MAC_REAL_UI_TRANSCRIPT"))
        for message in transcript where message.role == .assistant {
            let identity = try XCTUnwrap(message.journalReference).cursor.identity
            let terminal = try XCTUnwrap(cold.canonicalRunTerminal(
                conversationID: identity.conversation_id, runID: identity.run_id))
            XCTAssertTrue(terminal.result.value.kind == .answered || terminal.result.value.kind == .actionCompleted,
                "A visible ACK and sealed Run do not establish success: \(terminal.result.value.kind)")
        }
    }

    func testRealA2UIResumesPreservedRunWithOriginalModelIdentity() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1",
              let conversation = environment["NOLA_MAC_RESUME_CONVERSATION"],
              let runID = environment["NOLA_MAC_RESUME_RUN"],
              let endpointID = environment["NOLA_MAC_RESUME_ENDPOINT_ID"],
              let modelJSON = environment["NOLA_MAC_RESUME_MODEL_JSON"],
              let configuration = realLLMConfiguration(environment) else {
            throw XCTSkip("Requires the explicitly retained isolated Run and original provider binding")
        }
        let database = MacApplicationPort.defaultDatabaseURL
        guard MacAppEnvironment.contains(database, in: URL(fileURLWithPath: "/tmp")),
              environment["NOLA_TEST_MODE"] == "1", environment["NOLA_USE_REAL_KEYCHAIN_IN_TESTS"] == "0" else {
            throw MacApplicationPortError.invalidResponse
        }
        let originalModel = try JSONDecoder().decode(MacRemoteModelConfiguration.self, from: Data(modelJSON.utf8))
        XCTAssertEqual(originalModel.modelName, configuration.model)
        let wire = try XCTUnwrap(MacModelWireProtocol(rawValue: try XCTUnwrap(environment["NOLA_REAL_LLM_WIRE_PROTOCOL"])))
        // Restore the exact test-owned profile/credential identity removed by the
        // first fixture's teardown; never substitute the user's current selection.
        let port = try MacApplicationPort(databaseURL: database)
        let credentialAccount = try XCTUnwrap(environment["NOLA_MAC_RESUME_CREDENTIAL_ACCOUNT"])
        // XCTest's vault is deliberately process-local. Supply only the exact
        // retained profile account from the isolated fixture; runtime still
        // validates the original endpoint/protocol/model/secret digest.
        let credentialStore = MacAppleChatConfigurationStore(applicationPort: port, credentialReader: { account in
            account == credentialAccount ? configuration.apiKey : nil
        })
        let runtime = MacAgentHostRuntime(credentialStore: credentialStore)
        try credentialStore.replaceEndpoints([.init(id: endpointID,
            name: environment["NOLA_MAC_RESUME_ENDPOINT_NAME"] ?? "A2UI isolated acceptance",
            baseURL: configuration.endpoint, apiKey: configuration.apiKey, models: [originalModel], wireProtocol: wire)])
        let before = try port.listUISurfaces(conversationID: conversation)
        let originalUserIDs = try port.loadConversationWindow(conversation).projection.messages.filter { $0.role == .user }.map(\.messageID)
        let identity = try XCTUnwrap(before.first?.identity)
        let repository = MacCanonicalConversationRepository(store: port)
        let model = MacWorkspaceModel(initialConversationID: conversation, canonicalRepository: repository,
            localAgentRuntime: runtime, remoteHost: PersistenceTestRemoteHost(), modelApplication: port)
        model.executionLocation = .thisMac
        model.selectedConversationID = conversation
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let root = MacChatWorkspaceView(initialConversationID: conversation, forcesInspectorVisible: false,
            canonicalRepository: repository, localAgentRuntime: runtime,
            remoteHost: PersistenceTestRemoteHost(),
            inspectorDependencies: MacInspectorDependencies(dataRegistry: MacDataSourceRegistry.shared, platformData: MacPlatformDataService.shared,
                webSources: MacWebSourceStore.shared, chromeMessaging: MacChromeMessagingService.shared), model: model)
            .frame(width: 1100, height: 800)
        let hosting = NSHostingView(rootView: root)
        hosting.frame = NSRect(x: 0, y: 0, width: 1100, height: 800)
        let window = NSWindow(contentRect: hosting.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        await settle(hosting)
        let recoveryDeadline = ContinuousClock.now + .seconds(20)
        while model.localRunRecovery?.expectedRunID != runID && ContinuousClock.now < recoveryDeadline {
            try await Task.sleep(for: .milliseconds(50))
        }
        XCTAssertEqual(model.localRunRecovery?.expectedRunID, runID)
        let active = try XCTUnwrap(port.activeRunIdentity(conversationID: conversation))
        let exact = try XCTUnwrap(port.loadExactRunInvocation(conversationID: conversation,
            expectedRunID: runID, assistantMessageID: try XCTUnwrap(active.assistantMessageID)))
        let frozen = try MacFrozenLaunchDescriptor.load(exact)
        let bookmarks = MacSecurityScopedBookmarkStore.shared
        let workspace = try await bookmarks.resolveSubmissionSelection(bookmarks.captureSubmissionSelection(
            conversationID: conversation, selectedURL: model.workspaceURL))
        let rules = try port.effectivePermissionRules(conversationID: conversation, hostID: "mac-local")
        let currentRules = try JSONDecoder().decode([MacCanonicalJSONValue].self, from: JSONEncoder().encode(rules.rules))
        let skills = try MacBuiltInSkillRegistry.activeDescriptors(store: port)
        let diagnostics: [String: Any] = [
            "frozen_project": frozen.project_path, "current_project": workspace.projectURL.resolvingSymlinksInPath().path,
            "frozen_workspace_paths": frozen.authorized_workspace_paths,
            "current_workspace_paths": workspace.authorizedURLs.map { $0.resolvingSymlinksInPath().path },
            "frozen_permission_mode": frozen.permission_mode, "current_permission_mode": model.permissionMode.rawValue,
            "frozen_rule_count": frozen.permission_rules.count, "current_rule_count": currentRules.count,
            "rules_equal": frozen.permission_rules == currentRules,
            "frozen_skill_ids": frozen.active_skill_ids, "current_skill_ids": skills.map { $0.id.rawValue },
            "frozen_allowed_arguments": frozen.allowed_directory_arguments,
            "current_builtin_allowed_arguments": MacBuiltInSkillRegistry.allowedDirectoryArguments(projectURL: workspace.projectURL, activeSkills: skills),
            "frozen_skill_arguments": frozen.skill_directory_arguments,
            "current_builtin_skill_arguments": MacBuiltInSkillRegistry.skillDirectoryArguments(activeSkills: skills)
        ]
        if let path = environment["NOLA_MAC_RESUME_DIAGNOSTICS"] {
            try JSONSerialization.data(withJSONObject: diagnostics, options: [.prettyPrinted, .sortedKeys])
                .write(to: URL(fileURLWithPath: path), options: .atomic)
        }
        if environment["NOLA_MAC_RESUME_INSPECT_ONLY"] == "1" { return }
        let checkingDeadline = ContinuousClock.now + .seconds(30)
        while model.isCheckingLocalRunRecovery && ContinuousClock.now < checkingDeadline {
            try await Task.sleep(for: .milliseconds(50))
        }
        await model.checkLocalRunRecovery()
        while model.isCheckingLocalRunRecovery && ContinuousClock.now < checkingDeadline {
            try await Task.sleep(for: .milliseconds(50))
        }
        guard model.canResumeLocalRun else {
            XCTFail(model.localRecoveryError ?? "Original Run is not resumable; checking=\(model.isCheckingLocalRunRecovery), unknownEffects=\(model.currentUnknownEffectRecovery != nil)")
            throw MacLocalAgentResumeError.snapshotUnavailable
        }
        await model.resumeLocalRun()
        var timeline: [[String: Any]] = []
        var approvals: Set<String> = []
        try await observeRun(stage: "a2ui-original-cold-resume", model: model, hosting: hosting,
            timeline: &timeline, permissionRequestIDs: &approvals, timeout: .seconds(480),
            timelineURL: try artifactURL(environment, "NOLA_MAC_REAL_UI_TIMELINE"),
            screenshotURL: try artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT"),
            accessibilityURL: try artifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY"), requireIncrementalText: false)
        let finalProjection = try port.loadConversationWindow(conversation).projection
        XCTAssertNil(finalProjection.activeRun)
        XCTAssertEqual(finalProjection.messages.filter { $0.role == .user }.map(\.messageID), originalUserIDs,
            "Cold resume must not synthesize another user input or resubmit the UI action")
        let after = try XCTUnwrap(port.listUISurfaces(conversationID: conversation).first { $0.identity == identity })
        XCTAssertTrue(after.nodes.values.contains { $0.props["text"]?.journalString?.hasPrefix("ACK ORCHID") == true })
        XCTAssertEqual(after.identity, identity)
    }

    func testRealActivePermissionPolicySwitchesBothWaysWithoutCrossingConversationOwners() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Use the isolated real Mac XCTest runner")
        }
        let config = (endpoint: try XCTUnwrap(environment["NOLA_REAL_LLM_ENDPOINT"]),
            apiKey: try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"]),
            model: try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"]))
        struct DeclaredModel: Decodable {
            let model: String
            let wireProtocol: MacModelWireProtocol
            let executionCapabilities: MacModelExecutionCapabilities
        }
        let declaredJSON = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL_FIXTURE_JSON"])
        let declared = try JSONDecoder().decode(DeclaredModel.self, from: Data(declaredJSON.utf8))
        let runnerProtocol = try XCTUnwrap(environment["NOLA_REAL_LLM_WIRE_PROTOCOL"])
        XCTAssertEqual(declared.model, config.model)
        XCTAssertEqual(declared.wireProtocol.rawValue, runnerProtocol)
        guard declared.model == config.model,
              declared.wireProtocol.rawValue == runnerProtocol else {
            throw MacApplicationPortError.invalidResponse
        }
        let databasePath = try XCTUnwrap(environment["NOLA_DB_PATH"])
        let screenshot = try artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT")
        let testRoot = screenshot.deletingLastPathComponent().deletingLastPathComponent()
            .standardizedFileURL.resolvingSymlinksInPath()
        let database = URL(fileURLWithPath: databasePath).standardizedFileURL.resolvingSymlinksInPath()
        let isolated = environment["NOLA_TEST_MODE"] == "1"
            && environment["NOLA_USE_REAL_KEYCHAIN_IN_TESTS"] == "0"
            && testRoot.lastPathComponent.hasPrefix("nola-mac-real-memory.")
            && MacAppEnvironment.contains(testRoot, in: URL(fileURLWithPath: "/tmp"))
            && MacAppEnvironment.contains(database, in: testRoot)
            && MacApplicationPort.defaultDatabaseURL.standardizedFileURL.resolvingSymlinksInPath() == database
        XCTAssertTrue(isolated, "Never run permission acceptance against user storage or credentials")
        guard isolated else { throw MacApplicationPortError.invalidResponse }

        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let configurationStore = MacAppleChatConfigurationStore.shared
        let original = configurationStore.endpoints()
        try configurationStore.replaceEndpoints([MacRemoteEndpointConfiguration(
            name: "Isolated policy acceptance", baseURL: config.endpoint, apiKey: config.apiKey,
            models: [MacRemoteModelConfiguration(modelName: config.model,
                executionCapabilities: declared.executionCapabilities)], wireProtocol: declared.wireProtocol)])
        defer { try? configurationStore.replaceEndpoints(original) }
        let port = try MacApplicationPort(databaseURL: database)
        // A separate production connection observes facts; it never applies a
        // policy, claims a notice or manufactures a Host acknowledgement.
        let cold = try MacApplicationPort(databaseURL: database)
        let repository = MacCanonicalConversationRepository(store: port)
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: MacAgentHostRuntime.shared, remoteHost: MacRemoteHostService.shared,
            modelApplication: port)
        model.executionLocation = .thisMac
        model.newConversation()
        let other = try XCTUnwrap(model.selectedConversationID)
        model.draft = "POLICY-B-UNSENT"
        let otherMetadata = try XCTUnwrap(cold.loadConversationMetadata(other))
        XCTAssertEqual(model.permissionMode, .cautious)
        XCTAssertNil(try cold.activeRunIdentity(conversationID: other))
        model.newConversation()
        let conversation = try XCTUnwrap(model.selectedConversationID)
        XCTAssertNotEqual(conversation, other)
        let root = MacChatWorkspaceView(initialConversationID: conversation, forcesInspectorVisible: true,
            canonicalRepository: repository, localAgentRuntime: MacAgentHostRuntime.shared,
            remoteHost: MacRemoteHostService.shared, inspectorDependencies: MacInspectorDependencies(
                dataRegistry: MacDataSourceRegistry.shared,
                platformData: MacPlatformDataService.shared, webSources: MacWebSourceStore.shared,
                chromeMessaging: MacChromeMessagingService.shared), model: model)
            .frame(width: 1440, height: 900).environment(\.colorScheme, .dark)
        let hosting = NSHostingView(rootView: root)
        hosting.frame = NSRect(x: 0, y: 0, width: 1440, height: 900)
        let window = NSWindow(contentRect: hosting.bounds, styleMask: [.titled, .resizable],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.orderFrontRegardless()
        defer {
            if model.isRunning { model.stop(reason: "isolated policy acceptance cleanup") }
            model.presentationDidDetach()
            window.orderOut(nil); window.contentView = nil; window.close()
        }
        await settle(hosting)
        await model.loadSelectedConversationIfNeeded()

        func waitUntil(_ purpose: String, timeout: Duration = .seconds(30),
                       condition: () throws -> Bool) async throws {
            let deadline = ContinuousClock.now + timeout
            repeat {
                try Task.checkCancellation()
                if try condition() { return }
                try await Task.sleep(for: .milliseconds(100))
            } while ContinuousClock.now < deadline
            XCTFail("Timed out: \(purpose)")
            throw NSError(domain: "NolaMacTests.RealPermissionPolicy", code: 1,
                userInfo: [NSLocalizedDescriptionKey: purpose])
        }
        var timeline: [[String: Any]] = []
        var policyEvidence: [[String: Any]] = []
        let timelineURL = try artifactURL(environment, "NOLA_MAC_REAL_UI_TIMELINE")
        let policyURL = screenshot.deletingLastPathComponent().appendingPathComponent("active-permission-policy.json")
        func record(_ stage: String) throws {
            timeline.append(["stage": stage, "running": model.isRunning,
                "selected": model.selectedConversationID ?? "", "permissionProfile": model.permissionMode.rawValue,
                "permissionState": String(describing: model.selectedPermissionConfigurationState),
                "assistantCharacters": model.selectedMessages.last(where: { $0.role == .assistant })?.text.count ?? 0])
            try persistTimeline(timeline, to: timelineURL)
        }

        // A real long answer supplies the active window. If it finishes before
        // either exact ACK, fail rather than treating an idle save as success.
        model.draft = "不使用工具。写80条不同的程序调试建议，编号分行，每条至少40个汉字。最后单独写 NOLA-POLICY-END。"
        let send = try XCTUnwrap(accessibilityElement(hosting, identifier: "mac-composer-send"))
        XCTAssertEqual(send.accessibilityPerformPress?(), true)
        var admitted: MacApplicationPort.ActiveRunIdentity?
        try await waitUntil("real Host admission and first streamed body", timeout: .seconds(90)) {
            admitted = try cold.activeRunIdentity(conversationID: conversation)
            return admitted?.assistantMessageID != nil
                && model.selectedMessages.last(where: { $0.role == .assistant })?.journalReference != nil
                && model.selectedMessages.last(where: { $0.role == .assistant })?.text.isEmpty == false
        }
        let owner = try XCTUnwrap(admitted)
        let assistant = try XCTUnwrap(owner.assistantMessageID)
        let initialMetadata = try XCTUnwrap(cold.loadConversationMetadata(conversation))
        var revision = try XCTUnwrap(initialMetadata.configurationRevision)
        model.draft = "POLICY-A-UNSENT"
        try record("admitted")

        for (index, mode) in [MacPermissionMode.allowAll, .cautious].enumerated() {
            let current = try XCTUnwrap(cold.activeRunIdentity(conversationID: conversation))
            XCTAssertEqual(current.runID, owner.runID)
            XCTAssertEqual(current.assistantMessageID, assistant)
            XCTAssertEqual(current.invocationConfigDigest, owner.invocationConfigDigest)
            guard try cold.canonicalRunTerminal(conversationID: conversation, runID: owner.runID) == nil else {
                XCTFail("Both policy changes must occur on the same formally nonterminal Run")
                throw MacApplicationPortError.invalidResponse
            }
            let expectedRevision = revision + 1
            XCTAssertTrue(model.selectComposerPermissionMode(mode, conversationID: conversation))
            try record("policy-\(index)-issued")
            // Deliberately leave before the asynchronous CAS/ACK completes.
            model.selectedConversationID = other
            await model.loadSelectedConversationIfNeeded()
            try await waitUntil("B permission owner settles independently") {
                model.selectedPermissionConfigurationState == .saved
            }
            XCTAssertEqual(model.selectedConversationID, other)
            XCTAssertEqual(model.permissionMode, .cautious)
            XCTAssertNil(model.selectedPermissionActiveMode)
            XCTAssertEqual(model.draft, "POLICY-B-UNSENT")
            XCTAssertFalse(model.selectComposerPermissionMode(mode, conversationID: conversation),
                "A stale action must not edit another selected conversation")
            try await model.waitForModelConfigurationCommit(conversationID: conversation)
            var ack: MacActivePermissionPolicy?
            try await waitUntil("exact production Host ACK for \(mode.rawValue)") {
                let active = try cold.activeRunIdentity(conversationID: conversation)
                guard active?.runID == owner.runID, active?.assistantMessageID == assistant,
                      try cold.canonicalRunTerminal(conversationID: conversation, runID: owner.runID) == nil else {
                    throw MacApplicationPortError.invalidResponse
                }
                ack = try cold.loadPermissionProjection(conversation, runID: owner.runID)?.activePolicy
                let after = try cold.activeRunIdentity(conversationID: conversation)
                guard after?.runID == owner.runID, after?.assistantMessageID == assistant,
                      after?.invocationConfigDigest == owner.invocationConfigDigest,
                      try cold.canonicalRunTerminal(conversationID: conversation, runID: owner.runID) == nil else {
                    throw MacApplicationPortError.invalidResponse
                }
                return ack?.runID == owner.runID && ack?.profile == mode
                    && ack?.configurationRevision == expectedRevision
            }
            let applied = try XCTUnwrap(ack)
            XCTAssertEqual(try cold.loadConversationMetadata(conversation)?.configurationRevision, expectedRevision)
            XCTAssertEqual(try cold.loadConversationMetadata(other), otherMetadata)
            XCTAssertNil(try cold.activeRunIdentity(conversationID: other))
            XCTAssertEqual(model.selectedConversationID, other)
            XCTAssertEqual(model.permissionMode, .cautious)
            XCTAssertEqual(model.selectedPermissionConfigurationState, .saved)
            XCTAssertNil(model.selectedPermissionActiveMode)
            XCTAssertEqual(model.draft, "POLICY-B-UNSENT")
            policyEvidence.append(["conversation_id": conversation, "run_id": applied.runID,
                "assistant_message_id": assistant, "configuration_revision": applied.configurationRevision,
                "profile": applied.profile.rawValue, "selected_during_ack": other,
                "authority": "cold_application_permission_projection"])
            try JSONSerialization.data(withJSONObject: policyEvidence, options: [.prettyPrinted, .sortedKeys])
                .write(to: policyURL, options: .atomic)
            try record("policy-\(index)-ack-while-B-selected")
            await settle(hosting)
            try capturePNG(hosting, to: screenshot.deletingLastPathComponent().appendingPathComponent("policy-\(index)-B.png"))
            model.selectedConversationID = conversation
            await model.loadSelectedConversationIfNeeded()
            try await waitUntil("A composer consumes its own formal ACK") {
                model.selectedPermissionConfigurationState == .active && model.selectedPermissionActiveMode == mode
            }
            XCTAssertEqual(model.permissionMode, mode)
            XCTAssertEqual(model.draft, "POLICY-A-UNSENT")
            XCTAssertNotNil(accessibilityElement(hosting, identifier: "mac-composer-permission-status"))
            try record("policy-\(index)-A-active")
            revision = expectedRevision
        }
        XCTAssertEqual(policyEvidence.count, 2)
        try await waitUntil("real Host normal answer seals", timeout: .seconds(240)) {
            guard !model.isRunning else { return false }
            return try cold.canonicalRunTerminal(conversationID: conversation, runID: owner.runID) != nil
        }
        await settle(hosting)
        let terminal = try XCTUnwrap(cold.canonicalRunTerminal(conversationID: conversation, runID: owner.runID))
        XCTAssertEqual(terminal.result.value.kind, .answered)
        XCTAssertEqual(terminal.assistantMessageID, assistant)
        XCTAssertTrue(model.selectedMessages.last(where: { $0.role == .assistant })?.text.contains("NOLA-POLICY-END") == true)
        XCTAssertEqual(try cold.loadConversationMetadata(other), otherMetadata)
        try record("completed")
        try capturePNG(hosting, to: screenshot)
        try accessibilityTree(hosting).write(to: artifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY"),
            atomically: true, encoding: .utf8)
        try persistCanonicalTranscript(model.selectedMessages,
            to: artifactURL(environment, "NOLA_MAC_REAL_UI_TRANSCRIPT"))
        model.selectedConversationID = other
        await model.loadSelectedConversationIfNeeded()
        XCTAssertEqual(model.draft, "POLICY-B-UNSENT")
        XCTAssertEqual(model.permissionMode, .cautious)
        XCTAssertNil(model.selectedPermissionActiveMode)
    }

    func testRealStreamingSurvivesConversationSwitchAndKeepsLatestVisible() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard let config = realLLMConfiguration(environment) else {
            throw XCTSkip("Real provider configuration is required")
        }
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let database = MacApplicationPort.defaultDatabaseURL
        // XCTest receives the public runtime variable names, not the runner's
        // NOLA_MAC_E2E_* shell variables; its own TMPDIR may also be different.
        let configuredDatabase = try XCTUnwrap(environment["NOLA_DB_PATH"])
        let screenshotPath = try XCTUnwrap(environment["NOLA_MAC_REAL_UI_SCREENSHOT"])
        let testRoot = URL(fileURLWithPath: screenshotPath).deletingLastPathComponent().deletingLastPathComponent()
        let isolated = testRoot.lastPathComponent.hasPrefix("nola-mac-real-memory.")
            && MacAppEnvironment.contains(testRoot, in: URL(fileURLWithPath: "/tmp"))
            && database.standardizedFileURL == URL(fileURLWithPath: configuredDatabase).standardizedFileURL
            && MacAppEnvironment.contains(database, in: testRoot)
        XCTAssertTrue(isolated, "Real UI acceptance requires the runner's isolated database")
        guard isolated else { return }
        let original = MacAppleChatConfigurationStore.shared.endpoints()
        try MacAppleChatConfigurationStore.shared.replaceEndpoints([.init(
            id: "streaming-real-provider", name: "Owned streaming acceptance", baseURL: config.endpoint,
            apiKey: config.apiKey, models: [.init(id: "streaming-real-model", modelName: config.model,
                executionCapabilities: .init(toolCalls: true, reasoning: true, samplingParameters: false))],
            wireProtocol: try XCTUnwrap(MacModelWireProtocol(rawValue:
                environment["NOLA_REAL_LLM_WIRE_PROTOCOL"] ?? ""), "Explicit real-provider protocol is required"))])
        defer { try? MacAppleChatConfigurationStore.shared.replaceEndpoints(original) }
        let repository = MacCanonicalConversationRepository(store: try MacApplicationPort(databaseURL: database))
        var conversation = "stream-switch-\(UUID().uuidString)"
        let model = MacWorkspaceModel(initialConversationID: conversation, canonicalRepository: repository,
            localAgentRuntime: MacAgentHostRuntime.shared, remoteHost: PersistenceTestRemoteHost())
        model.executionLocation = .thisMac
        model.selectedConversationID = conversation
        let root = MacChatWorkspaceView(initialConversationID: conversation, forcesInspectorVisible: true,
            canonicalRepository: repository, localAgentRuntime: MacAgentHostRuntime.shared,
            remoteHost: PersistenceTestRemoteHost(), inspectorDependencies: MacInspectorDependencies(
                dataRegistry: MacDataSourceRegistry.shared,
                platformData: MacPlatformDataService.shared, webSources: MacWebSourceStore.shared,
                chromeMessaging: MacChromeMessagingService.shared), model: model)
            .frame(width: 1440, height: 900).environment(\.colorScheme, .dark)
        let hosting = NSHostingView(rootView: root)
        hosting.frame = NSRect(x: 0, y: 0, width: 1440, height: 900)
        let window = NSWindow(contentRect: hosting.bounds, styleMask: [.titled, .resizable],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.orderFrontRegardless()
        defer {
            if model.isRunning { model.stop(reason: "isolated test cleanup") }
            window.orderOut(nil); window.contentView = nil; window.close()
        }
        await settle(hosting)
        model.newConversation()
        conversation = try XCTUnwrap(model.selectedConversationID)
        await model.loadSelectedConversationIfNeeded()
        await settle(hosting)
        try await model.waitForModelConfigurationCommit()
        model.draft = "不使用工具。用中文写60条不同的程序调试建议，每条至少30字，编号分行。最后单独写 NOLA-STREAM-END。"
        await settle(hosting)
        let sendButton = try XCTUnwrap(accessibilityElement(hosting, identifier: "mac-composer-send"))
        XCTAssertEqual(sendButton.isAccessibilityEnabled?(), true)
        XCTAssertEqual(sendButton.accessibilityPerformPress?(), true)
        XCTAssertTrue(model.isRunning, "Submission not admitted: selected=\(model.selectedConversationID ?? "nil"), exists=\(model.conversations.contains { $0.id == conversation }), activity=\(model.activity)")
        // Fault-injection acceptance includes two real generations. Keep its
        // explicit functional budget separate from the ordinary 240 s gate.
        let configuredDeadline = TimeInterval(environment["NOLA_MAC_STREAM_TEST_DEADLINE_SECONDS"] ?? "") ?? 240
        let deadline = Date().addingTimeInterval(min(480, max(1, configuredDeadline)))
        var switched = false
        var draftConversation: String?
        var sampledLengths: Set<Int> = []
        let screenshot = try artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT")
        let timelineURL = try artifactURL(environment, "NOLA_MAC_REAL_UI_TIMELINE")
        var timeline: [[String: Any]] = []
        var nextAccessibilitySample = SuspendingClock.now
        var lastAccessibilityLength = 0
        repeat {
            try await Task.sleep(for: .milliseconds(100))
            // Full AX traversal resolves attributed text throughout SwiftUI.
            // Sampling it every 100 ms can dominate the main actor and distort
            // the very streaming interaction this test observes. Keep cheap
            // state samples frequent, with independently paced AX snapshots.
            let sampledAccessibility = SuspendingClock.now >= nextAccessibilitySample
            if sampledAccessibility {
                lastAccessibilityLength = accessibilityTree(hosting).count
                sampledLengths.insert(lastAccessibilityLength)
                nextAccessibilitySample = SuspendingClock.now + .seconds(1)
            }
            timeline.append(["time": Date().timeIntervalSince1970, "running": model.isRunning,
                             "selected": model.selectedConversationID ?? "", "axCharacters": lastAccessibilityLength,
                             "axSampled": sampledAccessibility])
            if !switched, model.isRunning,
               model.selectedMessages.last(where: { $0.role == .assistant })?.journalReference != nil {
                switched = true
                model.newConversation()
                draftConversation = model.selectedConversationID
                model.draft = "UNSENT-DRAFT"
                await settle(hosting)
                XCTAssertFalse(model.isSelectedConversationVisiblyRunning)
                XCTAssertFalse(model.selectedComposerState.isRunning)
                XCTAssertNil(accessibilityElement(hosting, identifier: "mac-composer-stop"))
                let inactiveSend = try XCTUnwrap(accessibilityElement(hosting, identifier: "mac-composer-send"))
                XCTAssertEqual(inactiveSend.isAccessibilityEnabled?(), !model.selectedComposerState.isPrimaryActionDisabled)
                if model.selectedComposerState.isSubmissionBlocked {
                    XCTAssertNotNil(accessibilityElement(hosting, identifier: "chat.composer.submission-block-reason"),
                        "A disabled send action must explain the actual cross-conversation admission constraint")
                }
                try capturePNG(hosting, to: screenshot.deletingLastPathComponent().appendingPathComponent("new-conversation-during-stream.png"))
                model.selectedConversationID = conversation
                await model.loadSelectedConversationIfNeeded()
                await settle(hosting)
                XCTAssertEqual(model.selectedConversationID, conversation)
                XCTAssertEqual(model.draft, "", "The running conversation must not inherit the other conversation's draft")
                if model.isSelectedConversationVisiblyRunning {
                    XCTAssertNotNil(accessibilityElement(hosting, identifier: "mac-composer-stop"))
                }
            }
        } while model.isRunning && Date() < deadline
        try persistTimeline(timeline, to: timelineURL)
        try capturePNG(hosting, to: screenshot)
        try accessibilityTree(hosting).write(to: try artifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY"),
                                            atomically: true, encoding: .utf8)
        try JSONSerialization.data(withJSONObject: [
            "activity": String(describing: model.activity),
            "localRecoveryError": model.localRecoveryError ?? "",
            "localRecovery": String(describing: model.localRunRecovery),
            "messages": model.selectedMessages.map { String(describing: $0) }
        ], options: [.prettyPrinted]).write(to: screenshot.deletingLastPathComponent().appendingPathComponent("stream-terminal-diagnostics.json"))
        XCTAssertFalse(model.isRunning, "Real provider turn exceeded the bounded test deadline")
        XCTAssertTrue(switched, "The test must actually switch during a live request")
        XCTAssertGreaterThan(sampledLengths.count, 3, "The real UI must publish incremental states")
        await settle(hosting)
        XCTAssertEqual(model.selectedMessages.last(where: { $0.role == .assistant })?.deliveryState, .completed,
                       "Visible text and an idle composer are not proof of a successfully committed answer")
        let completedTree = accessibilityTree(hosting)
        XCTAssertTrue(completedTree.contains("NOLA-STREAM-END"),
                      "The completed real answer must survive switching and remain in the product view")
        // An end marker and a completed Run do not prove a coherent answer:
        // an interrupted response can remain visible ahead of a restarted list.
        // This is an assertion for this explicitly numbered test prompt, not
        // production duplicate detection or a language-based routing rule.
        let numberedItem = try NSRegularExpression(
            pattern: #"(?m)^\s*AXStaticText id=mac-assistant-response-bubble label= value=(\d+)\.$"#)
        let treeRange = NSRange(completedTree.startIndex..., in: completedTree)
        let visibleNumbers = numberedItem.matches(in: completedTree, range: treeRange).compactMap { match -> Int? in
            guard let range = Range(match.range(at: 1), in: completedTree) else { return nil }
            return Int(completedTree[range])
        }
        XCTAssertEqual(visibleNumbers, Array(1...60),
                       "Stream recovery must not leave an abandoned prefix before a restarted answer")
        func views(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap(views) }
        let footer = try XCTUnwrap(views(hosting).compactMap { $0 as? MacTranscriptLatestAnchor.AnchorView }.first)
        let scroll = try XCTUnwrap(footer.enclosingScrollView)
        let document = try XCTUnwrap(scroll.documentView)
        XCTAssertEqual(scroll.documentVisibleRect.maxY, footer.convert(footer.bounds, to: document).maxY,
                       accuracy: 3, "Terminal reflow must not strand the viewport below the answer")
        XCTAssertNil(accessibilityElement(hosting, identifier: "mac-composer-stop"))
        XCTAssertNotNil(accessibilityElement(hosting, identifier: "mac-composer-send"))
        try persistCanonicalTranscript(model.selectedMessages,
            to: artifactURL(environment, "NOLA_MAC_REAL_UI_TRANSCRIPT"))
        model.selectedConversationID = try XCTUnwrap(draftConversation)
        await model.loadSelectedConversationIfNeeded()
        await settle(hosting)
        XCTAssertEqual(model.draft, "UNSENT-DRAFT", "Background completion must preserve the other conversation's draft")
        XCTAssertNil(accessibilityElement(hosting, identifier: "mac-composer-stop"))
        XCTAssertEqual(accessibilityElement(hosting, identifier: "mac-composer-send")?.isAccessibilityEnabled?(), true)
    }

    private func accessibilityElement(_ root: NSView, identifier: String) -> AnyObject? {
        var queue: [AnyObject] = [root]
        var visited = Set<ObjectIdentifier>()
        var cursor = 0
        while cursor < queue.count && visited.count < 4_000 {
            let element = queue[cursor]
            cursor += 1
            guard visited.insert(ObjectIdentifier(element)).inserted else { continue }
            if element.accessibilityIdentifier?() == identifier { return element }
            let children: [Any] = (element.accessibilityChildren?() ?? nil) ?? []
            queue.append(contentsOf: children.map { $0 as AnyObject })
            if let view = element as? NSView { queue.append(contentsOf: view.subviews) }
        }
        return nil
    }

    func testRealSearchThenWordWorkflowRendersPersistsAndRestoresInProductUI() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard let realConfiguration = realLLMConfiguration(environment) else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 to run the real Mac product UI chain.")
        }
        let endpoint = realConfiguration.endpoint
        let apiKey = realConfiguration.apiKey
        let modelID = realConfiguration.model
        // Application and Host derive their store from the same app environment.
        // Changing NOLA_DB_PATH after singleton initialization cannot isolate
        // either one. Refuse persistent user state before saving configuration.
        let databaseURL = MacApplicationPort.defaultDatabaseURL
        _ = try XCTUnwrap(
            MacAppEnvironment.contains(
                databaseURL,
                in: FileManager.default.temporaryDirectory
            ) ? databaseURL : nil,
            "Real product tests require a temporary NOLA_MAC_APP_SUPPORT set before app launch."
        )
        let screenshotURL = try artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT")
        let accessibilityURL = try artifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY")
        let timelineURL = try artifactURL(environment, "NOLA_MAC_REAL_UI_TIMELINE")
        let transcriptURL = try artifactURL(environment, "NOLA_MAC_REAL_UI_TRANSCRIPT")

        let workspace = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-product-ui-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: workspace, withIntermediateDirectories: true)
        let workspaceEvidenceURL = timelineURL.deletingLastPathComponent()
            .appendingPathComponent("mac-product-ui-workspace.txt")
        var reachedSuccessfulEnd = false
        defer {
            if reachedSuccessfulEnd, testRun?.failureCount == 0 {
                try? FileManager.default.removeItem(at: workspace)
            } else {
                try? workspace.path.write(
                    to: workspaceEvidenceURL,
                    atomically: true,
                    encoding: .utf8
                )
            }
        }

        let previousConfiguration = MacAppleChatConfigurationStore.shared.endpoints()
        try MacAppleChatConfigurationStore.shared.save(
            baseURL: endpoint,
            apiKey: apiKey,
            model: modelID,
            executionCapabilities: .init(toolCalls: true, samplingParameters: true)
        )
        defer {
            try? MacAppleChatConfigurationStore.shared.replaceEndpoints(previousConfiguration)
        }

        let conversationID = "mac-product-ui-\(UUID().uuidString)"
        // The product UI and Agent Host must share the app-scoped canonical
        // store; a separate repository would reconcile against empty history.
        let applicationPort = try MacApplicationPort(
            databaseURL: databaseURL
        )
        let conversationRepository = MacCanonicalConversationRepository(store: applicationPort)
        let workspaceModel = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: conversationRepository,
            localAgentRuntime: MacAgentHostRuntime.shared,
            remoteHost: PersistenceTestRemoteHost()
        )
        workspaceModel.executionLocation = .thisMac
        workspaceModel.workspaceURL = workspace
        workspaceModel.selectedConversationID = conversationID

        let root = MacChatWorkspaceView(
            initialConversationID: conversationID,
            forcesInspectorVisible: true,
            canonicalRepository: conversationRepository,
            localAgentRuntime: MacAgentHostRuntime.shared,
            remoteHost: PersistenceTestRemoteHost(),
            inspectorDependencies: MacInspectorDependencies(
                dataRegistry: MacDataSourceRegistry.shared,
                platformData: MacPlatformDataService.shared,
                webSources: MacWebSourceStore.shared,
                chromeMessaging: MacChromeMessagingService.shared
            ),
            model: workspaceModel
        )
        .frame(width: 1_440, height: 900)
        .environment(\.colorScheme, .dark)
        let hosting = NSHostingView(rootView: root)
        hosting.frame = NSRect(x: 0, y: 0, width: 1_440, height: 900)
        hosting.appearance = NSAppearance(named: .darkAqua)
        let window = NSWindow(
            contentRect: hosting.bounds,
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.animationBehavior = .none
        window.contentView = hosting
        window.orderFrontRegardless()
        defer {
            window.orderOut(nil)
            window.contentView = nil
        }
        await settle(hosting)

        // Keep the continuity marker short and deterministic. Long random UUID
        // markers turn this product-flow test into a model transcription test;
        // real providers occasionally alter one hex digit even when the chat,
        // tools, persistence, and rendering are all correct.
        let marker = "NOLA-MAC-REAL-CHAIN-E2E"
        var timeline: [[String: Any]] = []
        var permissionRequestIDs: Set<String> = []

        workspaceModel.draft = "查找目前仍可报名的人工智能比赛，使用可靠网页来源核对报名状态、截止日期和官方链接。给出简洁中文表格，并在结尾保留标记 \(marker)。"
        workspaceModel.send()
        try await observeRun(
            stage: "search",
            model: workspaceModel,
            hosting: hosting,
            timeline: &timeline,
            permissionRequestIDs: &permissionRequestIDs,
            timeout: .seconds(600),
            timelineURL: timelineURL,
            screenshotURL: screenshotURL,
            accessibilityURL: accessibilityURL
        )

        let firstAssistant = try XCTUnwrap(
            workspaceModel.messagesByConversation[conversationID]?
                .last(where: { $0.role == .assistant })
        )
        XCTAssertFalse(firstAssistant.text.isEmpty)
        let firstPresentation = try XCTUnwrap(firstAssistant.presentation)
        let firstNodeIDs = firstPresentation.renderedTextNodes.map(\.id)
        XCTAssertFalse(firstNodeIDs.isEmpty, "Nonempty assistant output must have rendered nodes.")
        XCTAssertEqual(
            Set(firstNodeIDs).count,
            firstNodeIDs.count,
            "The first completed turn must not replay an already committed render node."
        )
        XCTAssertFalse(firstAssistant.text.contains("Local agent error:"))
        XCTAssertFalse(firstAssistant.text.contains("DSML"), "Tool protocol leaked into the first assistant body.")
        XCTAssertFalse(firstAssistant.text.contains("<tool_calls>"), "Tool protocol leaked into the first assistant body.")
        XCTAssertFalse(firstAssistant.text.contains(String(localized: "Nola could not produce a final answer. Please try again.")))
        guard firstAssistant.text.contains(marker) else {
            XCTFail("The search turn did not complete with the requested marker. Response: \(firstAssistant.text)")
            return
        }
        XCTAssertTrue(
            firstAssistant.presentation?.traceItems.contains {
                $0.kind == .tool && $0.title == "web_search" && $0.state == .completed
            } == true,
            "The real product turn did not retain a completed web_search trace."
        )
        let firstTools = firstAssistant.presentation?.traceItems.filter { $0.kind == .tool } ?? []
        XCTAssertFalse(firstTools.isEmpty)
        XCTAssertFalse(firstAssistant.text.hasPrefix("I have enough"))
        XCTAssertFalse(firstAssistant.text.contains("Let me verify"))
        let disqualifyingStatusEvidence = [
            "报名通道已关闭",
            "报名通道刚关闭",
            "截止日期临近或已过",
            "部分高校 7 月已截止",
            "未公布具体截止日期",
            "截止日期未明确",
            "以各校通知为准",
            "全国报名已进入后期",
        ]
        let tableDataRows = firstAssistant.text
            .split(separator: "\n")
            .map(String.init)
            .filter { line in
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                return trimmed.hasPrefix("|")
                    && !trimmed.contains("---")
                    && !trimmed.contains("比赛名称")
            }
        for row in tableDataRows {
            for evidence in disqualifyingStatusEvidence + ["已截止", "未明确"] {
                XCTAssertFalse(
                    row.contains(evidence),
                    "A currently-open-only table contains a rejected or unresolved row: \(row)"
                )
            }
        }
        if firstAssistant.text.contains("未能确认任何") {
            let recommendationLines = firstAssistant.text
                .split(separator: "\n")
                .map { $0.trimmingCharacters(in: .whitespaces) }
                .filter { $0.hasPrefix("- ") }
            XCTAssertTrue(
                recommendationLines.isEmpty,
                "A no-match answer must not reintroduce rejected candidates as recommendations: \(recommendationLines)"
            )
        }

        let outputName = "AI赛事_\(marker).docx"
        let firstTurnHasNoQualifyingItems = firstAssistant.text.contains("未找到报名通道仍开放")
            || firstAssistant.text.contains("未能确认任何")
        workspaceModel.draft = "把上一轮已经核对的结论原样整理成可编辑 Word 文档，文件名必须是 \(outputName)。使用内置 Word Skill，保留来源链接和截止日期；不得重新引入上一轮已排除的项目，也不得补充未经新证据确认的事实。如果上一轮没有符合条件的比赛，文档必须明确记录没有符合项。成功验证文档后只报告最终文件。"
        workspaceModel.send()
        try await observeRun(
            stage: "word",
            model: workspaceModel,
            hosting: hosting,
            timeline: &timeline,
            permissionRequestIDs: &permissionRequestIDs,
            timeout: .seconds(600),
            timelineURL: timelineURL,
            screenshotURL: screenshotURL,
            accessibilityURL: accessibilityURL
        )

        let messages = try XCTUnwrap(workspaceModel.messagesByConversation[conversationID])
        XCTAssertEqual(
            messages.map(\.role),
            [.user, .assistant, .user, .assistant],
            "The product transcript must preserve both user turns and their assistant replies in order."
        )
        XCTAssertTrue(messages[0].text.contains(marker))
        XCTAssertTrue(messages[2].text.contains(outputName))
        let assistants = messages.filter { $0.role == .assistant }
        XCTAssertEqual(assistants.count, 2, "Each user turn must own exactly one assistant bubble.")
        let finalAssistant = try XCTUnwrap(assistants.last)
        XCTAssertGreaterThan(
            workspaceModel.contextUsed,
            0,
            "The completed product UI published terminal state before authoritative context usage."
        )
        XCTAssertGreaterThan(
            workspaceModel.contextWindowTokens,
            0,
            "The completed product UI lost the configured model context window."
        )
        XCTAssertFalse(finalAssistant.text.isEmpty)
        let finalPresentation = try XCTUnwrap(finalAssistant.presentation)
        let finalNodeIDs = finalPresentation.renderedTextNodes.map(\.id)
        XCTAssertFalse(finalNodeIDs.isEmpty, "Nonempty follow-up output must have rendered nodes.")
        XCTAssertEqual(
            Set(finalNodeIDs).count,
            finalNodeIDs.count,
            "Canonical completion must replace the render snapshot, not append its tail again."
        )
        XCTAssertFalse(finalAssistant.text.contains("Local agent error:"))
        XCTAssertFalse(finalAssistant.text.contains("DSML"), "Tool protocol leaked into the final assistant body.")
        XCTAssertFalse(finalAssistant.text.contains("<tool_calls>"), "Tool protocol leaked into the final assistant body.")
        XCTAssertFalse(finalAssistant.text.contains(String(localized: "Nola could not produce a final answer. Please try again.")))
        XCTAssertTrue(finalAssistant.text.contains(outputName))
        if firstTurnHasNoQualifyingItems {
            XCTAssertFalse(
                finalAssistant.text.contains("项仍可报名"),
                "A follow-up artifact contradicted the owned prior-turn conclusion by inventing qualifying items: \(finalAssistant.text)"
            )
        }
        XCTAssertTrue(
            finalAssistant.presentation?.traceItems.contains {
                $0.kind == .tool && $0.title == "Skill" && $0.state == .completed
            } == true,
            "The Word turn did not expose its Skill context in the assistant process."
        )
        XCTAssertTrue(
            finalAssistant.presentation?.traceItems.contains {
                $0.kind == .tool && $0.title == "write_file" && $0.state == .completed
            } == true,
            "The Word turn did not expose the specification write."
        )
        XCTAssertFalse(
            finalAssistant.presentation?.traceItems.contains {
                $0.kind == .tool && $0.state == .failed
            } == true,
            "The successful Word turn retained a failed tool operation."
        )

        let documentURL = workspace.appendingPathComponent(outputName)
        XCTAssertTrue(FileManager.default.fileExists(atPath: documentURL.path))
        try assertValidWordPackage(documentURL)
        XCTAssertTrue(
            permissionRequestIDs.isEmpty,
            "Recommended Mac permissions must not interrupt non-destructive workspace search and creation: \(permissionRequestIDs)"
        )

        await settle(hosting)
        try capturePNG(hosting, to: screenshotURL)
        try persistCanonicalTranscript(messages, to: transcriptURL)
        let accessibility = accessibilityTree(hosting)
        try accessibility.write(to: accessibilityURL, atomically: true, encoding: .utf8)
        XCTAssertTrue(
            accessibility.contains(marker),
            "The rendered assistant answer is missing from the macOS accessibility tree."
        )
        XCTAssertTrue(
            accessibility.contains(outputName),
            "The rendered output artifact is missing from the macOS accessibility tree."
        )

        let encodedTimeline = try JSONSerialization.data(
            withJSONObject: timeline,
            options: [.prettyPrinted, .sortedKeys]
        )
        try encodedTimeline.write(to: timelineURL, options: .atomic)
        try await Task.sleep(for: .milliseconds(900))
        let restored = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: conversationRepository,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        let restoredMessages = try XCTUnwrap(restored.messagesByConversation[conversationID])
        XCTAssertEqual(restoredMessages.map(\.id), messages.map(\.id))
        XCTAssertEqual(restoredMessages.filter { $0.role == .assistant }.count, 2)
        XCTAssertEqual(restoredMessages.last?.text, finalAssistant.text)
        XCTAssertFalse(restoredMessages.last?.presentation?.traceItems.isEmpty ?? true)
        reachedSuccessfulEnd = true
    }

    func testRealTransformerResearchCppAndWordWorkflowInProductUI() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard let realConfiguration = realLLMConfiguration(environment) else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 to run the real Transformer product UI chain.")
        }
        let endpoint = realConfiguration.endpoint
        let apiKey = realConfiguration.apiKey
        let modelID = realConfiguration.model
        let previousDatabasePath = environment["NOLA_DB_PATH"]
        setenv("NOLA_DB_PATH", realConfiguration.databasePath, 1)
        defer {
            if let previousDatabasePath {
                setenv("NOLA_DB_PATH", previousDatabasePath, 1)
            } else {
                unsetenv("NOLA_DB_PATH")
            }
        }
        let screenshotURL = try artifactURL(environment, "NOLA_MAC_REAL_UI_SCREENSHOT")
        let accessibilityURL = try artifactURL(environment, "NOLA_MAC_REAL_UI_ACCESSIBILITY")
        let timelineURL = try artifactURL(environment, "NOLA_MAC_REAL_UI_TIMELINE")
        let transcriptURL = try artifactURL(environment, "NOLA_MAC_REAL_UI_TRANSCRIPT")

        let workspace = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-transformer-ui-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: workspace, withIntermediateDirectories: true)
        let workspaceEvidenceURL = timelineURL.deletingLastPathComponent()
            .appendingPathComponent("mac-transformer-workspace.txt")
        var reachedSuccessfulEnd = false
        defer {
            if reachedSuccessfulEnd, testRun?.failureCount == 0 {
                try? FileManager.default.removeItem(at: workspace)
            } else {
                try? workspace.path.write(to: workspaceEvidenceURL, atomically: true, encoding: .utf8)
            }
        }

        let previousConfiguration = MacAppleChatConfigurationStore.shared.endpoints()
        try MacAppleChatConfigurationStore.shared.save(baseURL: endpoint, apiKey: apiKey, model: modelID,
            executionCapabilities: .init(toolCalls: true, samplingParameters: true))
        defer {
            try? MacAppleChatConfigurationStore.shared.replaceEndpoints(previousConfiguration)
        }

        let conversationID = "mac-transformer-ui-\(UUID().uuidString)"
        let applicationPort = try MacApplicationPort(databaseURL: MacApplicationPort.defaultDatabaseURL)
        let repository = MacCanonicalConversationRepository(store: applicationPort)
        let workspaceModel = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: repository,
            localAgentRuntime: MacAgentHostRuntime.shared,
            remoteHost: PersistenceTestRemoteHost()
        )
        workspaceModel.executionLocation = .thisMac
        workspaceModel.workspaceURL = workspace
        workspaceModel.selectedConversationID = conversationID

        let root = MacChatWorkspaceView(
            initialConversationID: conversationID,
            forcesInspectorVisible: true,
            canonicalRepository: repository,
            localAgentRuntime: MacAgentHostRuntime.shared,
            remoteHost: PersistenceTestRemoteHost(),
            inspectorDependencies: MacInspectorDependencies(
                dataRegistry: MacDataSourceRegistry.shared,
                platformData: MacPlatformDataService.shared,
                webSources: MacWebSourceStore.shared,
                chromeMessaging: MacChromeMessagingService.shared
            ),
            model: workspaceModel
        )
        .frame(width: 1_560, height: 980)
        .environment(\.colorScheme, .dark)
        let hosting = NSHostingView(rootView: root)
        hosting.frame = NSRect(x: 0, y: 0, width: 1_560, height: 980)
        hosting.appearance = NSAppearance(named: .darkAqua)
        let window = NSWindow(
            contentRect: hosting.bounds,
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.animationBehavior = .none
        window.contentView = hosting
        window.orderFrontRegardless()
        defer {
            window.orderOut(nil)
            window.contentView = nil
        }
        await settle(hosting)

        let marker = "NOLA-MAC-TRANSFORMER-E2E"
        let sourceName = "transformer_reference.cpp"
        let documentName = "Transformer算法与C++实现.docx"
        var timeline: [[String: Any]] = []
        var permissionRequestIDs: Set<String> = []

        workspaceModel.draft = """
        完成一项可交付的 Transformer 算法工程任务。先联网检索并阅读可靠的一手资料，至少包括《Attention Is All You Need》原始论文和一个权威实现或框架文档；核对缩放点积注意力、多头注意力、位置编码、LayerNorm、前馈网络和残差连接。然后在当前工作区创建 \(sourceName)：只使用 C++17 标准库，实现可实际运行的 Transformer encoder 参考程序，包含稳定 softmax、scaled dot-product attention、multi-head attention、sin/cos 位置编码、LayerNorm、FFN、残差连接、维度检查和确定性演示；必须真实编译并运行验证。最后使用内置 Word Skill 创建可编辑的 \(documentName)，文档须包含人工智能背景、算法结构、关键公式说明、实现取舍、复杂度、测试结果、完整 C++ 源码、编译运行命令，以及检索来源的真实可点击链接；至少包含一个原生 Word 表格。不要只在回复里粘贴内容，两个文件都必须生成并验证。最终简洁报告文件和验证结果，并保留标记 \(marker)。
        """
        workspaceModel.send()
        try await observeRun(
            stage: "transformer",
            model: workspaceModel,
            hosting: hosting,
            timeline: &timeline,
            permissionRequestIDs: &permissionRequestIDs,
            timeout: .seconds(900),
            timelineURL: timelineURL,
            screenshotURL: screenshotURL,
            accessibilityURL: accessibilityURL
        )

        let messages = try XCTUnwrap(workspaceModel.messagesByConversation[conversationID])
        XCTAssertEqual(messages.map(\.role), [.user, .assistant])
        let assistant = try XCTUnwrap(messages.last)
        XCTAssertFalse(assistant.text.isEmpty)
        XCTAssertTrue(assistant.text.contains(marker), "The final answer lost the requested marker.")
        XCTAssertTrue(assistant.text.contains(sourceName), "The final answer did not expose the C++ artifact.")
        XCTAssertFalse(assistant.text.contains("DSML"))
        XCTAssertFalse(assistant.text.contains("<tool_calls>"))
        XCTAssertFalse(assistant.text.contains("Local agent error:"))

        let tools = assistant.presentation?.traceItems.filter { $0.kind == .tool } ?? []
        let normalizedToolTitles = tools.map { $0.title.lowercased() }
        XCTAssertTrue(
            normalizedToolTitles.contains { $0.contains("web_search") || $0.contains("联网搜索") },
            "The workflow did not expose its web research in the assistant process: \(normalizedToolTitles)"
        )
        XCTAssertTrue(
            normalizedToolTitles.contains { $0 == "skill" || $0.contains("技能") },
            "The workflow did not expose the Word Skill invocation: \(normalizedToolTitles)"
        )
        XCTAssertTrue(
            normalizedToolTitles.contains { $0.contains("write_file") || $0.contains("写入文件") },
            "The workflow did not expose file creation: \(normalizedToolTitles)"
        )
        XCTAssertTrue(
            normalizedToolTitles.contains { $0.contains("bash") || $0.contains("运行命令") },
            "The workflow did not expose compilation and execution: \(normalizedToolTitles)"
        )
        XCTAssertNotEqual(
            tools.last?.state,
            .failed,
            "The workflow ended on a failed tool instead of a successful delivery."
        )
        XCTAssertTrue(
            tools.contains { $0.state == .completed && $0.title.lowercased().contains("bash") },
            "The workflow never completed a real compile or execution step."
        )

        let sourceURL = workspace.appendingPathComponent(sourceName)
        let documentURL = workspace.appendingPathComponent(documentName)
        XCTAssertTrue(FileManager.default.fileExists(atPath: sourceURL.path), "C++ artifact is missing.")
        XCTAssertTrue(FileManager.default.fileExists(atPath: documentURL.path), "Word artifact is missing.")
        let source = try String(contentsOf: sourceURL, encoding: .utf8)
        for token in ["softmax", "attention", "LayerNorm", "MultiHead", "sin", "cos"] {
            XCTAssertTrue(source.localizedCaseInsensitiveContains(token), "C++ source is missing \(token).")
        }
        XCTAssertFalse(source.contains("TODO"), "C++ source still contains TODO placeholders.")

        let executableURL = workspace.appendingPathComponent("transformer_reference_e2e")
        let compile = Process()
        compile.executableURL = URL(fileURLWithPath: "/usr/bin/xcrun")
        compile.arguments = [
            "clang++", "-std=c++17", "-O2", "-Wall", "-Wextra", "-pedantic",
            sourceURL.path, "-o", executableURL.path,
        ]
        let compileError = Pipe()
        compile.standardError = compileError
        try compile.run()
        compile.waitUntilExit()
        let compilerDiagnostics = String(
            data: compileError.fileHandleForReading.readDataToEndOfFile(),
            encoding: .utf8
        ) ?? ""
        XCTAssertEqual(compile.terminationStatus, 0, "Independent C++ compilation failed: \(compilerDiagnostics)")

        let execute = Process()
        execute.executableURL = executableURL
        let standardOutput = Pipe()
        let standardError = Pipe()
        execute.standardOutput = standardOutput
        execute.standardError = standardError
        try execute.run()
        execute.waitUntilExit()
        let demoOutput = String(
            data: standardOutput.fileHandleForReading.readDataToEndOfFile(),
            encoding: .utf8
        ) ?? ""
        let demoError = String(
            data: standardError.fileHandleForReading.readDataToEndOfFile(),
            encoding: .utf8
        ) ?? ""
        XCTAssertEqual(execute.terminationStatus, 0, "Generated Transformer demo failed: \(demoError)")
        XCTAssertFalse(demoOutput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)

        try assertValidWordPackage(documentURL)
        let documentXML = try transformerZipEntry("word/document.xml", from: documentURL)
        let relationshipsXML = try transformerZipEntry("word/_rels/document.xml.rels", from: documentURL)
        for token in ["Transformer", "softmax", "LayerNorm", "C++17"] {
            XCTAssertTrue(documentXML.localizedCaseInsensitiveContains(token), "Word document is missing \(token).")
        }
        XCTAssertTrue(relationshipsXML.contains("TargetMode=\"External\""))
        XCTAssertTrue(
            relationshipsXML.localizedCaseInsensitiveContains("arxiv.org")
                || relationshipsXML.localizedCaseInsensitiveContains("doi.org"),
            "Word sources do not include a clickable primary-paper link."
        )
        XCTAssertTrue(permissionRequestIDs.isEmpty, "Recommended permissions interrupted a non-destructive workflow.")

        await settle(hosting)
        try capturePNG(hosting, to: screenshotURL)
        let accessibility = accessibilityTree(hosting)
        try accessibility.write(to: accessibilityURL, atomically: true, encoding: .utf8)
        XCTAssertTrue(accessibility.contains(marker))
        XCTAssertTrue(accessibility.contains(sourceName))
        XCTAssertTrue(accessibility.contains(documentName))

        try persistCanonicalTranscript(messages, to: transcriptURL)
        try JSONSerialization.data(withJSONObject: timeline, options: [.prettyPrinted, .sortedKeys])
            .write(to: timelineURL, options: .atomic)

        try await Task.sleep(for: .milliseconds(900))
        let restored = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        let restoredMessages = try XCTUnwrap(restored.messagesByConversation[conversationID])
        XCTAssertEqual(restoredMessages.map(\.id), messages.map(\.id))
        XCTAssertEqual(restoredMessages.last?.text, assistant.text)
        XCTAssertFalse(restoredMessages.last?.presentation?.traceItems.isEmpty ?? true)
        reachedSuccessfulEnd = true
    }

    private func transformerZipEntry(_ entry: String, from archive: URL) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/unzip")
        process.arguments = ["-p", archive.path, entry]
        let output = Pipe()
        let error = Pipe()
        process.standardOutput = output
        process.standardError = error
        try process.run()
        // Drain stdout while unzip is running. Waiting first deadlocks once a
        // realistic Word document's XML exceeds the pipe buffer: unzip waits
        // for a reader while XCTest waits for unzip to terminate.
        let outputData = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        let diagnostics = String(
            data: error.fileHandleForReading.readDataToEndOfFile(),
            encoding: .utf8
        ) ?? ""
        XCTAssertEqual(process.terminationStatus, 0, "Unable to read \(entry): \(diagnostics)")
        return String(
            data: outputData,
            encoding: .utf8
        ) ?? ""
    }

    private func observeRun(
        stage: String,
        model: MacWorkspaceModel,
        hosting: NSHostingView<some View>,
        timeline: inout [[String: Any]],
        permissionRequestIDs: inout Set<String>,
        timeout: Duration,
        timelineURL: URL,
        screenshotURL: URL,
        accessibilityURL: URL,
        requireIncrementalText: Bool = true
    ) async throws {
        let deadline = ContinuousClock.now + timeout
        var sampleIndex = 0
        var previousAssistantText = ""
        var sawPendingAssistantBubble = false
        var sawIncrementalAssistantGrowth = false
        var capturedVisualStates: Set<String> = []
        var previousTraceSignature = ""
        var previousTimelineSignature = ""
        let screenshotDirectory = screenshotURL.deletingLastPathComponent()
            .appendingPathComponent("mac-product-ui-screenshots", isDirectory: true)
        try FileManager.default.createDirectory(
            at: screenshotDirectory,
            withIntermediateDirectories: true
        )

        func captureVisualState(
            _ name: String,
            messages: [MacChatMessage],
            assistant: MacChatMessage?
        ) throws {
            let key = "\(stage)-\(name)"
            guard capturedVisualStates.insert(key).inserted else { return }
            let prefix = String(format: "%02d-%@-%@", capturedVisualStates.count, stage, name)
            let pngURL = screenshotDirectory.appendingPathComponent("\(prefix).png")
            let treeURL = screenshotDirectory.appendingPathComponent("\(prefix)-accessibility.txt")
            let stateURL = screenshotDirectory.appendingPathComponent("\(prefix)-state.json")
            try capturePNG(hosting, to: pngURL)
            try accessibilityTree(hosting).write(to: treeURL, atomically: true, encoding: .utf8)
            let state: [String: Any] = [
                "stage": stage,
                "state": name,
                "running": model.isRunning,
                "messageCount": messages.count,
                "messageRoles": messages.map { String(describing: $0.role) },
                "assistantID": assistant?.id.uuidString ?? "",
                "assistantCharacters": assistant?.text.count ?? 0,
                "assistantText": assistant?.text ?? "",
                "contextUsedTokens": model.contextUsed,
                "contextWindowTokens": model.contextWindowTokens,
                "contextUsageEstimated": model.contextUsageIsEstimated,
                "processTitle": assistant.map {
                    MacAssistantProcessTitlePolicy.title(
                        traces: $0.presentation?.traceItems ?? [],
                        isStreaming: model.isRunning
                    )
                } ?? "",
                "renderedTextNodes": assistant?.presentation?.renderedTextNodes.map {
                    [
                        "id": $0.id.uuidString,
                        "blockID": $0.blockID.uuidString,
                        "kind": String(describing: $0.kind),
                        "text": $0.text,
                    ]
                } ?? [],
                "traces": assistant?.presentation?.traceItems.map {
                    [
                        "kind": String(describing: $0.kind),
                        "title": $0.title,
                        "state": String(describing: $0.state),
                    ]
                } ?? [],
                "timestamp": ISO8601DateFormatter().string(from: .now),
            ]
            try JSONSerialization.data(withJSONObject: state, options: [.prettyPrinted, .sortedKeys])
                .write(to: stateURL, options: .atomic)
        }

        repeat {
            if let permission = model.pendingRemotePermission {
                if stage.hasPrefix("a2ui") {
                    guard permission.toolName == "render_ui" else {
                        model.resolvePermission(allow: false, scope: .once)
                        model.stop(reason: "Unexpected tool in isolated A2UI acceptance")
                        XCTFail("A2UI acceptance requested an unrelated permission: \(permission.toolName)")
                        throw MacApplicationPortError.invalidResponse
                    }
                    if !permissionRequestIDs.contains(permission.id),
                       let allow = accessibilityElement(hosting, identifier: "chat.permission.allow-once"),
                       allow.isAccessibilityEnabled?() == true {
                        XCTAssertEqual(allow.accessibilityPerformPress?(), true)
                        permissionRequestIDs.insert(permission.id)
                    }
                } else {
                    permissionRequestIDs.insert(String(describing: permission.id))
                    model.resolvePermission(allow: true, scope: .conversation)
                }
            }
            if sampleIndex % 3 == 0 || !model.isRunning {
                hosting.layoutSubtreeIfNeeded()
                hosting.displayIfNeeded()
                // Observe the exact canonical-plus-overlay projection rendered
                // by the product UI. The immediate assistant shell intentionally
                // exists before the durable Application projection is committed.
                let messages = model.selectedMessages
                let assistant = messages.last(where: { $0.role == .assistant })
                if assistant != nil, model.isRunning {
                    sawPendingAssistantBubble = true
                }
                if model.isRunning, assistant?.text.isEmpty != false {
                    try captureVisualState("submitted", messages: messages, assistant: assistant)
                }
                if let text = assistant?.text, !text.isEmpty {
                    guard text.hasPrefix(previousAssistantText) else {
                        throw NSError(
                            domain: "NolaMacTests.RealProductUIProjection",
                            code: 2,
                            userInfo: [
                                NSLocalizedDescriptionKey:
                                    "The live assistant body regressed or was replaced during the real \(stage) run."
                            ]
                        )
                    }
                    if !previousAssistantText.isEmpty && text.count > previousAssistantText.count {
                        sawIncrementalAssistantGrowth = true
                    }
                    if previousAssistantText.isEmpty {
                        try captureVisualState("first-body", messages: messages, assistant: assistant)
                    } else if text.count >= 240, previousAssistantText.count < 240 {
                        try captureVisualState("body-240", messages: messages, assistant: assistant)
                    } else if text.count >= 600, previousAssistantText.count < 600 {
                        try captureVisualState("body-600", messages: messages, assistant: assistant)
                    }
                    previousAssistantText = text
                }
        let traceItems = assistant?.presentation?.traceItems ?? []
        let recentTraces: [[String: String]] = traceItems.suffix(4).map { trace in
            let affectedFiles = (trace.affectedFiles ?? []).joined(separator: "\n")
            return [
                "id": trace.id.uuidString,
                "kind": String(describing: trace.kind),
                "title": trace.title,
                "state": String(describing: trace.state),
                "detail": String(trace.detail.prefix(512)),
                "arguments": String((trace.arguments ?? "").prefix(512)),
                "result": String((trace.result ?? "").prefix(512)),
                "affectedFiles": String(affectedFiles.prefix(1_024)),
            ]
        }
                let traceSignature = recentTraces.map {
                    "\($0["kind"] ?? "")|\($0["title"] ?? "")|\($0["state"] ?? "")"
                }.joined(separator: "\n")
                if !traceSignature.isEmpty, traceSignature != previousTraceSignature {
                    try captureVisualState(
                        "trace-\(assistant?.presentation?.traceItems.count ?? 0)",
                        messages: messages,
                        assistant: assistant
                    )
                    previousTraceSignature = traceSignature
                }
                let assistantCharacters = assistant?.text.count ?? 0
                let latestTrace = traceItems.last
                let timelineSignature = [
                    String(model.isRunning),
                    String(messages.count),
                    String(assistantCharacters),
                    String(traceItems.count),
                    latestTrace?.id.uuidString ?? "",
                    latestTrace.map { String(describing: $0.state) } ?? "",
                    String(model.pendingRemotePermission != nil),
                ].joined(separator: "|")
                if timelineSignature != previousTimelineSignature {
                    timeline.append([
                        "stage": stage,
                        "sample": sampleIndex,
                        "running": model.isRunning,
                        "messageCount": messages.count,
                        "assistantPresent": assistant != nil,
                        "assistantCharacters": assistantCharacters,
                        "renderedTextNodeCount": assistant?.presentation?.renderedTextNodes.count ?? 0,
                        "traceCount": traceItems.count,
                        "recentTraces": recentTraces,
                        "pendingPermission": model.pendingRemotePermission != nil,
                        "timestamp": ISO8601DateFormatter().string(from: .now),
                    ])
                    if timeline.count > 1_000 {
                        timeline.removeFirst(timeline.count - 1_000)
                    }
                    previousTimelineSignature = timelineSignature
                    try persistTimeline(timeline, to: timelineURL)
                }
            }
            if !model.isRunning { break }
            try await Task.sleep(for: .milliseconds(150))
            sampleIndex += 1
        } while ContinuousClock.now < deadline
        if model.localRunRecovery != nil, let error = model.localRecoveryError {
            let key = ProcessInfo.processInfo.environment["NOLA_REAL_LLM_API_KEY"] ?? ""
            let safeError = key.isEmpty ? error : error.replacingOccurrences(of: key, with: "[redacted]")
            XCTFail("Real UI execution entered unresolved recovery: \(safeError)")
            throw MacApplicationPortError.invalidResponse
        }
        if model.isRunning {
            // Record the actual unmet boundary before snapshot/teardown work.
            // XCTest can otherwise replace an async test's original error
            // with a secondary failed(deinit) transition during cleanup.
            let failure = "The real \(stage) product UI turn timed out while still running."
            XCTFail(failure)
            timeline.append([
                "stage": stage,
                "running": true,
                "failure": failure,
                "timestamp": ISO8601DateFormatter().string(from: .now),
            ])
            try persistTimeline(timeline, to: timelineURL)
            await settle(hosting)
            try capturePNG(hosting, to: screenshotURL)
            try accessibilityTree(hosting).write(
                to: accessibilityURL,
                atomically: true,
                encoding: .utf8
            )
            throw NSError(
                domain: "NolaMacTests.RealProductUITimeout",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "The real \(stage) product UI turn timed out."]
            )
        }
        guard sawPendingAssistantBubble else {
            throw NSError(
                domain: "NolaMacTests.RealProductUIProjection",
                code: 3,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        "The real \(stage) run did not render an assistant bubble immediately after submission."
                ]
            )
        }
        guard !requireIncrementalText || sawIncrementalAssistantGrowth else {
            throw NSError(
                domain: "NolaMacTests.RealProductUIProjection",
                code: 4,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        "The real \(stage) response never exposed more than one non-empty body state to the product UI."
                ]
            )
        }
        await settle(hosting)
        let completedMessages = model.selectedMessages
        try captureVisualState(
            "completed",
            messages: completedMessages,
            assistant: completedMessages.last(where: { $0.role == .assistant })
        )
    }

    private func persistTimeline(_ timeline: [[String: Any]], to url: URL) throws {
        try JSONSerialization.data(
            withJSONObject: timeline,
            options: [.prettyPrinted, .sortedKeys]
        ).write(to: url, options: .atomic)
    }

    private func realLLMConfiguration(
        _ environment: [String: String]
    ) -> (endpoint: String, apiKey: String, model: String, databasePath: String)? {
        let fixtureURL = URL(fileURLWithPath: "/tmp/nola-real-llm-e2e.json")
        let fixture = (try? Data(contentsOf: fixtureURL))
            .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: String] }
        guard environment["NOLA_REAL_LLM_E2E"] == "1" || fixture != nil else { return nil }
        let endpoint = environment["NOLA_REAL_LLM_ENDPOINT"] ?? fixture?["endpoint"] ?? ""
        let apiKey = environment["NOLA_REAL_LLM_API_KEY"] ?? fixture?["apiKey"] ?? ""
        let model = environment["NOLA_REAL_LLM_MODEL"] ?? fixture?["model"] ?? ""
        let databasePath = environment["NOLA_DB_PATH"]
            ?? fixture?["databasePath"]
            ?? MacApplicationPort.defaultDatabaseURL.path
        guard !endpoint.isEmpty, !apiKey.isEmpty, !model.isEmpty, !databasePath.isEmpty else {
            return nil
        }
        return (endpoint, apiKey, model, databasePath)
    }

    private func artifactURL(_ environment: [String: String], _ key: String) throws -> URL {
        let defaultNames = [
            "NOLA_MAC_REAL_UI_SCREENSHOT": "product-ui.png",
            "NOLA_MAC_REAL_UI_ACCESSIBILITY": "product-ui-accessibility.txt",
            "NOLA_MAC_REAL_UI_TIMELINE": "product-ui-timeline.json",
            "NOLA_MAC_REAL_UI_TRANSCRIPT": "product-ui-transcript.json",
        ]
        let defaultDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-real-ui-artifacts", isDirectory: true)
        let candidatePath = environment[key]
            ?? defaultNames[key].map { defaultDirectory.appendingPathComponent($0).path }
        let path = try XCTUnwrap(candidatePath, "Missing real UI artifact path: \(key)")
        let url = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        return url
    }

    private func capturePNG(_ hosting: NSView, to url: URL) throws {
        hosting.layoutSubtreeIfNeeded()
        hosting.displayIfNeeded()
        let representation = try XCTUnwrap(
            hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds)
        )
        hosting.cacheDisplay(in: hosting.bounds, to: representation)
        let data = try XCTUnwrap(representation.representation(using: .png, properties: [:]))
        let decoded = try XCTUnwrap(NSBitmapImageRep(data: data))
        XCTAssertGreaterThan(decoded.pixelsWide, 0)
        XCTAssertGreaterThan(decoded.pixelsHigh, 0)
        XCTAssertEqual(decoded.pixelsWide, representation.pixelsWide)
        XCTAssertEqual(decoded.pixelsHigh, representation.pixelsHigh)
        try data.write(to: url, options: .atomic)
    }

    private func accessibilityTree(_ root: NSView) -> String {
        var lines: [String] = []
        var visited: Set<ObjectIdentifier> = []
        func walk(_ seed: AnyObject, depth seedDepth: Int) {
            let maximumNodes = 4_000
            let maximumDepth = 24
            // SwiftUI nodes implement AX selectors without declaring the
            // NSAccessibilityProtocol conformance. Keep them and query through
            // Objective-C just as an accessibility client does.
            var queue: [(element: AnyObject, depth: Int)] = [(seed, seedDepth)]
            var cursor = 0

            while cursor < queue.count, visited.count < maximumNodes {
                let current = queue[cursor]
                cursor += 1
                let element = current.element
                let object = element
                let identity = ObjectIdentifier(object)
                guard visited.insert(identity).inserted else { continue }

                let identifier = element.accessibilityIdentifier?() ?? nil
                let label = element.accessibilityLabel?() ?? nil
                let rawValue: Any? = element.accessibilityValue?() ?? nil
                let value = rawValue.map { String(describing: $0) } ?? ""
                let role = (element.accessibilityRole?() ?? nil)?.rawValue ?? String(describing: type(of: element))
                lines.append("\(String(repeating: "  ", count: current.depth))\(role) id=\(identifier ?? "") label=\(label ?? "") value=\(value)")

                guard current.depth < maximumDepth else { continue }
                let raw: [Any] = (element.accessibilityChildren?() ?? nil) ?? []
                let children = raw + NSAccessibility.unignoredChildren(from: raw)
                for child in children {
                    queue.append((child as AnyObject, current.depth + 1))
                }
            }
        }
        walk(root, depth: 0)
        func walkViews(_ view: NSView, depth: Int) {
            walk(view, depth: depth)
            let identifier = view.accessibilityIdentifier()
            let label = view.accessibilityLabel() ?? ""
            let value = view.accessibilityValue().map { String(describing: $0) } ?? ""
            lines.append("\(String(repeating: "  ", count: depth))view=\(type(of: view)) id=\(identifier) label=\(label) value=\(value)")
            for subview in view.subviews {
                walkViews(subview, depth: depth + 1)
            }
        }
        walkViews(root, depth: 0)
        return lines.joined(separator: "\n")
    }

    private func assertValidWordPackage(_ url: URL) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/unzip")
        process.arguments = ["-t", url.path]
        let output = Pipe()
        process.standardOutput = output
        process.standardError = output
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)
        let listing = Process()
        listing.executableURL = URL(fileURLWithPath: "/usr/bin/unzip")
        listing.arguments = ["-Z1", url.path]
        let listingOutput = Pipe()
        listing.standardOutput = listingOutput
        try listing.run()
        listing.waitUntilExit()
        let entries = String(
            data: listingOutput.fileHandleForReading.readDataToEndOfFile(),
            encoding: .utf8
        ) ?? ""
        XCTAssertTrue(entries.contains("word/document.xml"))
        XCTAssertTrue(entries.contains("[Content_Types].xml"))
        let documentXML = try unzipEntry("word/document.xml", from: url)
        XCTAssertTrue(documentXML.contains("<w:tbl"), "The generated Word document must contain a real editable table.")
        XCTAssertTrue(documentXML.contains("人工智能"), "The generated Word document lost the requested subject matter.")
        let relationships = try unzipEntry("word/_rels/document.xml.rels", from: url)
        XCTAssertTrue(
            relationships.contains("TargetMode=\"External\"") && relationships.contains("http"),
            "The generated Word document must preserve clickable source links."
        )
    }

    private func unzipEntry(_ entry: String, from archive: URL) throws -> String {
        try transformerZipEntry(entry, from: archive)
    }

    private func settle(_ hosting: NSView) async {
        for _ in 0..<4 {
            hosting.layoutSubtreeIfNeeded()
            hosting.displayIfNeeded()
            try? await Task.sleep(for: .milliseconds(30))
            await Task.yield()
        }
    }

    // The Python gate compares this production-validated result with the same
    // persisted seal. It never infers completion from legacy Parts or Trace.
    private func persistCanonicalTranscript(_ messages: [MacChatMessage], to url: URL) throws {
        try MacCanonicalTranscriptEvidence.persist(messages, to: url)
    }
}
