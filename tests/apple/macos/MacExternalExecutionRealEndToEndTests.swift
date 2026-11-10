// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies installed external execution units through the production Mac Agent Host boundary.

import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacExternalExecutionRealEndToEndTests: XCTestCase {
    private static let realE2EMarkerPath = "/tmp/nola-run-real-mac-external-e2e"

    func testExecutionBackendCatalogIncludesStableBuiltInChoices() async throws {
        let catalog = try await MacLocalExecutionBackendCatalog.fetch()

        XCTAssertEqual(catalog.first?.id, "nola")
        XCTAssertEqual(
            Set(catalog.map(\.id)),
            Set(["nola", "codex", "claude_code", "opencode"])
        )
        if let codex = catalog.first(where: { $0.id == "codex" }), codex.isAvailable {
            XCTAssertFalse(
                codex.models.isEmpty,
                "An available Codex execution unit must expose its app-server model catalog."
            )
            XCTAssertNotNil(codex.defaultModelID)
        }
        XCTAssertTrue(catalog.first { $0.id == "nola" }?.models.isEmpty == true)
    }

    func testInstalledExternalExecutionUnitsRunAndResumeThroughMacHost() async throws {
        let environmentEnabled = ProcessInfo.processInfo.environment["NOLA_RUN_REAL_MAC_EXTERNAL_E2E"] == "1"
        let markerEnabled = FileManager.default.fileExists(atPath: Self.realE2EMarkerPath)
        guard environmentEnabled || markerEnabled else {
            throw XCTSkip(
                "Set NOLA_RUN_REAL_MAC_EXTERNAL_E2E=1 or create \(Self.realE2EMarkerPath) to run installed execution units."
            )
        }
        defer {
            if markerEnabled { try? FileManager.default.removeItem(atPath: Self.realE2EMarkerPath) }
        }

        let catalog = try await MacLocalExecutionBackendCatalog.fetch()
        let artifactRoot = ProcessInfo.processInfo.environment["NOLA_EXTERNAL_E2E_ARTIFACT_ROOT"]
        let root = artifactRoot.map { URL(fileURLWithPath: $0, isDirectory: true) }
            ?? FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-external-e2e-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { if artifactRoot == nil { try? FileManager.default.removeItem(at: root) } }

        let environment = MacAppEnvironment(
            applicationSupportRoot: root.appendingPathComponent("ApplicationSupport", isDirectory: true),
            cachesRoot: root.appendingPathComponent("Caches", isDirectory: true)
        )
        let applicationPort = try MacApplicationPort(
            databaseURL: root.appendingPathComponent("nola-product.db")
        )
        let conversationRepository = MacCanonicalConversationRepository(store: applicationPort)
        let defaultsName = "NolaMacTests.external-execution.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: defaultsName))
        defer { defaults.removePersistentDomain(forName: defaultsName) }
        let runtime = MacAgentHostRuntime()

        let markerBackends = markerEnabled
            ? (try? String(contentsOfFile: Self.realE2EMarkerPath, encoding: .utf8))
            : nil
        let backendSelection = markerBackends?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
            ? markerBackends
            : ProcessInfo.processInfo.environment["NOLA_EXTERNAL_EXECUTOR_BACKENDS"]
        let requestedBackends = backendSelection?
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        let backends = requestedBackends?.isEmpty == false
            ? requestedBackends!
            : ["codex", "claude_code", "opencode"]
        for backend in backends {
            let selected = try XCTUnwrap(catalog.first { $0.id == backend }, "Unknown backend: \(backend)")
            guard selected.isAvailable else {
                XCTFail(selected.diagnostic ?? "\(backend) is unavailable")
                return
            }
            let conversationID = "mac-external-\(backend)-\(UUID().uuidString)"
            let firstToken = "MAC_\(backend.uppercased())_FIRST_\(UUID().uuidString)"
            let first = try await run(
                runtime: runtime,
                backend: backend,
                conversationID: conversationID,
                token: firstToken,
                requireToolUse: true,
                externalSessionID: nil,
                root: root,
                environment: environment,
                canonicalRepository: conversationRepository,
                defaults: defaults
            )
            XCTAssertEqual(
                occurrences(of: firstToken, in: first.text),
                1,
                "\(backend) first response was: \(first.text)"
            )
            XCTAssertFalse(first.externalSessionID.isEmpty)
            XCTAssertTrue(first.observedToolStart, "\(backend) did not expose its tool start through Mac Agent Host")
            XCTAssertTrue(first.observedToolFinish, "\(backend) did not expose its tool completion through Mac Agent Host")

            let secondToken = "MAC_\(backend.uppercased())_RESUME_\(UUID().uuidString)"
            let second = try await run(
                runtime: runtime,
                backend: backend,
                conversationID: conversationID,
                token: secondToken,
                requireToolUse: true,
                externalSessionID: first.externalSessionID,
                root: root,
                environment: environment,
                canonicalRepository: conversationRepository,
                defaults: defaults
            )
            XCTAssertEqual(
                occurrences(of: secondToken, in: second.text),
                1,
                "\(backend) resumed response was: \(second.text)"
            )
            XCTAssertEqual(second.externalSessionID, first.externalSessionID)
            XCTAssertTrue(second.observedToolStart, "\(backend) resumed tool start is missing")
            XCTAssertTrue(second.observedToolFinish, "\(backend) resumed tool completion is missing")
        }
    }

    private func run(
        runtime: MacAgentHostRuntime,
        backend: String,
        conversationID: String,
        token: String,
        requireToolUse: Bool,
        externalSessionID: String?,
        root: URL,
        environment: MacAppEnvironment,
        canonicalRepository: MacCanonicalConversationRepository,
        defaults: UserDefaults
    ) async throws -> (
        text: String,
        externalSessionID: String,
        observedToolStart: Bool,
        observedToolFinish: Bool
    ) {
        let prompt: String
        if requireToolUse {
            let fixtureURL = root.appendingPathComponent("external-tool-input-\(backend).txt")
            try token.write(to: fixtureURL, atomically: true, encoding: .utf8)
            prompt = "Use an available file-reading tool to read \(fixtureURL.path), then reply with the file contents exactly once. Do not infer or repeat the path."
        } else {
            prompt = "Reply with this token exactly once: \(token)"
        }
        let submissionID = UUID().uuidString
        let userMessageID = UUID()
        let canonicalAssistantMessageID = "\(submissionID):assistant"
        let databaseURL = root.appendingPathComponent("nola.sqlite")
        let request = try MacLocalAgentRunRequest(
            conversationID: conversationID,
            submissionID: submissionID,
            userMessageID: userMessageID.uuidString,
            assistantMessageID: canonicalAssistantMessageID,
            prompt: prompt,
            projectURL: root,
            attachmentURLs: [],
            executionBackend: backend,
            externalSessionID: externalSessionID,
            memoryEnabled: false,
            databaseURLOverride: databaseURL,
            appEnvironment: environment,
            applicationPort: try MacApplicationPort(
                databaseURL: root.appendingPathComponent("nola.sqlite")
            ),
        )
        var journalEvents: [MacLocalAgentEvent] = []
        var observedToolStart = false
        var observedToolFinish = false
        let stream = await runtime.run(request)
        for try await event in stream {
            journalEvents.append(event)
            switch event.kind {
            case .toolStarted:
                observedToolStart = true
            case .toolFinished:
                observedToolFinish = true
            case .failed:
                XCTFail(event.text)
            default:
                break
            }
        }
        let text = try await readCommittedAssistantText(journalEvents, databaseURL: databaseURL)
        let canonical = try MacApplicationPort(databaseURL: databaseURL)
        let terminal = try XCTUnwrap(canonical.canonicalRunTerminal(
            conversationID: conversationID, runID: submissionID))
        XCTAssertEqual(terminal.assistantMessageID, canonicalAssistantMessageID)
        XCTAssertTrue(terminal.result.value.kind == .answered || terminal.result.value.kind == .actionCompleted,
            "External execution must seal a successful result, not merely end its transport")
        let stored = try XCTUnwrap(canonical.loadConversation(conversationID))
        XCTAssertEqual(
            stored.messages.filter { $0.id == userMessageID.uuidString }.count,
            1,
            "\(backend) must persist the Mac user message identity exactly once."
        )
        XCTAssertEqual(
            stored.messages.filter { $0.id == canonicalAssistantMessageID }.count,
            1,
            "\(backend) must complete the original Mac assistant placeholder exactly once."
        )
        XCTAssertEqual(
            stored.messages.first { $0.id == canonicalAssistantMessageID }?.runID,
            submissionID
        )
        // Session continuity is a durable delegated-invocation fact, not a transient
        // presentation status event. Check the committed record and resume input.
        let nativeSessionID = try persistedSessionID(databaseURL: databaseURL,
            runID: submissionID, backend: backend, resumedSessionID: externalSessionID)
        return (text, nativeSessionID, observedToolStart, observedToolFinish)
    }

    private func persistedSessionID(databaseURL: URL, runID: String, backend: String,
                                    resumedSessionID: String?) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
        let escapedRunID = runID.replacingOccurrences(of: "'", with: "''")
        process.arguments = ["-readonly", "-json", databaseURL.path,
            "SELECT payload_json FROM run_events WHERE run_id='\(escapedRunID)' AND event_kind='delegated_invocation_changed' ORDER BY sequence"]
        let output = Pipe()
        process.standardOutput = output
        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)
        let rows = try JSONDecoder().decode([[String: String]].self, from: data)
        let records = try rows.map { row -> [String: Any] in
            let payload = try XCTUnwrap(row["payload_json"])
            return try XCTUnwrap(JSONSerialization.jsonObject(with: Data(payload.utf8)) as? [String: Any])
        }
        let completed = try XCTUnwrap(records.last)
        XCTAssertEqual(completed["state"] as? String, "succeeded")
        XCTAssertEqual(completed["backend_id"] as? String, backend)
        let sessionID = try XCTUnwrap(completed["backend_session_id"] as? String)
        XCTAssertFalse(sessionID.isEmpty)
        if let resumedSessionID {
            let prepared = try XCTUnwrap(records.first)
            XCTAssertEqual(prepared["state"] as? String, "prepared")
            XCTAssertEqual(prepared["backend_session_id"] as? String, resumedSessionID)
            XCTAssertEqual(sessionID, resumedSessionID)
        }
        return sessionID
    }

    private func occurrences(of needle: String, in haystack: String) -> Int {
        haystack.components(separatedBy: needle).count - 1
    }
}
