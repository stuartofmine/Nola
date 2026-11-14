// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Exercises every Mac built-in Skill through the real Agent Host process and tool loop.

import AppKit
import Darwin
import Foundation
import PDFKit
import XCTest
@testable import Nola

@MainActor
final class MacBuiltInSkillEndToEndTests: XCTestCase {
    private var journalTextByRoot: [String: String] = [:]
    private var journalReasoningByRoot: [String: String] = [:]
    override func setUp() {
        super.setUp()
        executionTimeAllowance = ProcessInfo.processInfo.environment["NOLA_REAL_LLM_E2E"] == "1"
            ? 300
            : 45
        try? MacBuiltInSkillRegistry.restoreBuiltInSkills()
    }

    override func tearDown() {
        try? MacBuiltInSkillRegistry.restoreBuiltInSkills()
        super.tearDown()
    }

    private static let executionGate = SkillEndToEndExecutionGate()

    func testWordSkillEndToEnd() async throws {
        try await runScenario(
            .word,
            prompt: "Create a polished Word project brief from the supplied source and include an accessibility review.",
            skillMarker: "tracked changes",
            qualityGate: "styles, tracked changes, accessibility, and rendered-page review"
        )
    }

    func testExcelSkillEndToEnd() async throws {
        try await runScenario(
            .excel,
            prompt: "Analyze the supplied sales table and prepare a spreadsheet report with checked formulas.",
            skillMarker: "recalculation",
            qualityGate: "row-count reconciliation, formula recalculation, external-link, and chart checks"
        )
    }

    func testPowerPointSkillEndToEnd() async throws {
        try await runScenario(
            .powerpoint,
            prompt: "Turn the supplied launch notes into an executive presentation with speaker guidance.",
            skillMarker: "speaker notes",
            qualityGate: "story spine, speaker notes, overflow, and every-slide render checks"
        )
    }

    func testPDFSkillEndToEnd() async throws {
        try await runScenario(
            .pdf,
            prompt: "Inspect the supplied PDF request and prepare a safe redaction and delivery plan.",
            skillMarker: "underlying content",
            qualityGate: "page manifest, underlying-content removal, encryption, and rendered-page checks"
        )
    }

    func testFigmaSkillEndToEnd() async throws {
        try await runScenario(
            .figma,
            prompt: "Audit the supplied Figma component snapshot and prepare an implementation-ready handoff.",
            skillMarker: "Auto Layout",
            qualityGate: "before snapshot, Auto Layout, variables, component blast radius, and advisory-state checks"
        )
    }

    func testDataAnalysisSkillEndToEnd() async throws {
        try await runScenario(
            .dataAnalysis,
            prompt: "Analyze the supplied experiment extract and report defensible findings and limitations.",
            skillMarker: "analysis contract",
            qualityGate: "analysis contract, row-count reconciliation, uncertainty, and causal-boundary checks"
        )
    }

    func testDeepResearchSkillEndToEnd() async throws {
        try await runScenario(
            .deepResearch,
            prompt: "Research the supplied topic and produce a source-backed briefing with conflicting claims separated.",
            skillMarker: "source ledger",
            qualityGate: "original-query preservation, source ledger, primary-source, and cross-check coverage"
        )
    }

    func testDocumentReviewSkillEndToEnd() async throws {
        try await runScenario(
            .documentReview,
            prompt: "Review the supplied agreement and return prioritized, anchored findings with proposed revisions.",
            skillMarker: "finding categories",
            qualityGate: "finding categories, page or section anchors, semantic-diff, and tracked-change checks"
        )
    }

    func testImageOCRSkillEndToEnd() async throws {
        try await runScenario(
            .imageOCR,
            prompt: "Extract the supplied scan into structured text while preserving uncertainty and reading order, then save the result to outputs/scan-transcription.json.",
            skillMarker: "reading order",
            qualityGate: "reading order, raw-versus-corrected text, OCR confidence, and uncertain-region checks"
        )
    }

    func testMeetingSkillEndToEnd() async throws {
        try await runScenario(
            .meetings,
            prompt: "Convert the supplied meeting notes into decisions, owners, deadlines, and follow-up actions.",
            skillMarker: "explicit decisions",
            qualityGate: "explicit decisions, action-item ownership, deadline, and speaker-attribution checks"
        )
    }

    func testScheduleSkillEndToEnd() async throws {
        try await runScenario(
            .scheduleTasks,
            prompt: "Plan the supplied work into a conflict-free schedule and show the commit preview.",
            skillMarker: "preview table",
            qualityGate: "preview table, timezone, conflict, persistent-identifier, and partial-write checks"
        )
    }

    func testKnowledgeMemorySkillEndToEnd() async throws {
        try await runScenario(
            .knowledgeMemory,
            prompt: "Prepare a consent-first proposal for remembering the supplied durable preference.",
            skillMarker: "explicit consent",
            qualityGate: "explicit consent, provenance, expiry, sensitivity, and recall-verification checks"
        )
    }

    func testFileOrganizationSkillEndToEnd() async throws {
        try await runScenario(
            .fileOrganization,
            prompt: "Plan a reversible organization of the supplied file inventory without modifying it yet.",
            skillMarker: "dry-run manifest",
            qualityGate: "dry-run manifest, symlink, collision, hash-equality, and rollback checks"
        )
    }

    func testAcademicReaderSkillEndToEnd() async throws {
        try await runScenario(
            .academicReader,
            prompt: "Read the supplied paper and produce a structured evidence map without inventing claims.",
            skillMarker: "Nola runtime contract",
            qualityGate: "research question, method, evidence, limitations, and claim-to-source checks"
        )
    }

    func testScientificWritingSkillEndToEnd() async throws {
        try await runScenario(
            .scientificWriting,
            prompt: "Draft a concise scientific section from the supplied evidence while preserving uncertainty.",
            skillMarker: "Nola runtime contract",
            qualityGate: "evidence traceability, uncertainty, structure, terminology, and citation-boundary checks"
        )
    }

    func testAcademicPolishingSkillEndToEnd() async throws {
        try await runScenario(
            .academicPolishing,
            prompt: "Polish the supplied academic passage without changing its scientific meaning.",
            skillMarker: "Nola runtime contract",
            qualityGate: "meaning preservation, terminology, grammar, tone, and change-summary checks"
        )
    }

    func testManuscriptReviewerSkillEndToEnd() async throws {
        try await runScenario(
            .manuscriptReviewer,
            prompt: "Review the supplied manuscript and separate fatal, major, and minor concerns.",
            skillMarker: "Nola runtime contract",
            qualityGate: "evidence anchors, severity, reproducibility, actionable revision, and scope checks"
        )
    }

    func testReviewerResponseSkillEndToEnd() async throws {
        try await runScenario(
            .reviewerResponse,
            prompt: "Prepare a point-by-point response to the supplied reviewer comments.",
            skillMarker: "Nola runtime contract",
            qualityGate: "comment coverage, response evidence, manuscript location, respectful tone, and unresolved-item checks"
        )
    }

    func testStatisticsAuditSkillEndToEnd() async throws {
        try await runScenario(
            .statisticsAudit,
            prompt: "Audit the supplied statistical claims and report assumptions and uncertainty.",
            skillMarker: "Nola runtime contract",
            qualityGate: "estimand, assumptions, effect size, uncertainty, multiplicity, and reproducibility checks"
        )
    }

    func testReferenceVerifierSkillEndToEnd() async throws {
        try await runScenario(
            .referenceVerifier,
            prompt: "Verify the supplied references and distinguish verified, mismatched, and unresolved entries.",
            skillMarker: "Nola runtime contract",
            qualityGate: "identifier, metadata, claim support, retraction status, and unresolved-reference checks"
        )
    }

    func testPaperToPresentationSkillEndToEnd() async throws {
        try await runScenario(
            .paperToPresentation,
            prompt: "Turn the supplied paper into a concise research presentation with source-linked speaker notes.",
            skillMarker: "Nola runtime contract",
            qualityGate: "storyline, claim traceability, figure provenance, speaker notes, and slide-density checks"
        )
    }

    func testAutomationSkillEndToEnd() async throws {
        await Self.executionGate.acquire()
        do {

        let fileManager = FileManager.default
        let workspace = fileManager.temporaryDirectory
            .appendingPathComponent("nola-automation-e2e-\(UUID().uuidString)", isDirectory: true)
        let outputURL = workspace.appendingPathComponent("outputs/automation-run.json")
        try fileManager.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        defer { try? fileManager.removeItem(at: workspace) }

        let skillsRoot = try XCTUnwrap(MacBuiltInSkillRegistry.resourceRootURL)
        let designSkillRoot = skillsRoot.appendingPathComponent(
            MacBuiltInSkillID.automations.resourceDirectory,
            isDirectory: true
        )
        let executionSkillRoot = skillsRoot.appendingPathComponent(
            MacBuiltInSkillID.dataAnalysis.resourceDirectory,
            isDirectory: true
        )
        let prompt = "Design a reliable automation for the supplied workflow and keep it disabled until validated."
        let automationSourceID = UUID().uuidString.lowercased()
        let automationArguments: [String: Any] = [
            "name": "Daily fixture verification",
            "prompt": "Load the data-analysis Skill and write the validated automation fixture to outputs/automation-run.json.",
            "skill_id": MacBuiltInSkillID.dataAnalysis.rawValue,
            "frequency": "daily",
            "weekday": 1,
            "hour": 9,
            "minute": 0,
            "enabled": false,
            "run_when_data_changes": false,
            "context_sources": ["web_sources"],
            "execution_spec": ["kind": "agent", "spec": [
                "prompt": "Load the data-analysis Skill and write the validated automation fixture to outputs/automation-run.json.",
                "execution_scope": ["kind": "local_workspace", "root": workspace.path],
                "permission_profile": "cautious", "model_selection": "use_current_default_at_admission",
                "context_refs": [["kind": "web_source", "id": automationSourceID]],
                "skill_refs": [MacBuiltInSkillID.dataAnalysis.rawValue],
            ]],
            "web_sources": [[
                "id": automationSourceID,
                "name": "Orbital standards feed",
                "url": "__MOCK_SOURCE_BASE__/source/primary",
                "format": "text",
                "refresh_frequency": "daily",
            ]],
            "maximum_data_age_minutes": 60,
            "timeout_minutes": 2,
            "retry_count": 2,
        ]
        let outputContent = """
        {"status":"validated","idempotency_key":"automation-e2e-7301","rows":73}
        """
        let isolatedEnvironment = MacAppEnvironment(
            applicationSupportRoot: workspace.appendingPathComponent("ApplicationSupport", isDirectory: true),
            cachesRoot: workspace.appendingPathComponent("Caches", isDirectory: true)
        )
        let mockServer = try SkillEndToEndMockServer(
            configuration: [
                "prompt": prompt,
                "skill_root": designSkillRoot.path,
                "skill_name": MacBuiltInSkillID.automations.resourceDirectory,
                "skill_id": MacBuiltInSkillID.automations.rawValue,
                "skill_marker": "idempotency",
                "automation_arguments": automationArguments,
                "execution_skill_root": executionSkillRoot.path,
                "execution_skill_name": MacBuiltInSkillID.dataAnalysis.resourceDirectory,
                "execution_skill_id": MacBuiltInSkillID.dataAnalysis.rawValue,
                "workspace_path": workspace.path,
                "execution_output_path": "outputs/automation-run.json",
                "execution_output_absolute_path": outputURL.path,
                "execution_output_content": outputContent,
                "automation_persistence_path": isolatedEnvironment.databaseURL.path,
            ],
            workspace: workspace
        )
        defer { mockServer.stop() }

        let automationDirectory = isolatedEnvironment.supportRoot
            .appendingPathComponent("Automations", isDirectory: true)
        let launchAgentDirectory = workspace.appendingPathComponent("LaunchAgents", isDirectory: true)
        var activityStore = MacActivityStore(
            baseDirectory: isolatedEnvironment.supportRoot
                .appendingPathComponent("Activity", isDirectory: true)
        )
        var dataSourceRegistry = MacDataSourceRegistry(
            storeURL: isolatedEnvironment.supportRoot
                .appendingPathComponent("DataSources", isDirectory: true)
                .appendingPathComponent("source-registry.json")
        )
        var webSourceStore = MacWebSourceStore(
            storeURL: isolatedEnvironment.supportRoot
                .appendingPathComponent("WebSources", isDirectory: true)
                .appendingPathComponent("web-sources.json"),
            registry: dataSourceRegistry
        )
        var automationStore = try MacAutomationStore(
            baseDirectory: automationDirectory,
            launchAgentDirectory: launchAgentDirectory,
            synchronizeSystemJobs: false,
            appEnvironment: isolatedEnvironment,
            authorizationResolver: { MacAppEnvironment.contains($0, in: workspace) },
            activityStore: activityStore,
            completionNotifier: { _, _ in },
            dataSourceRegistry: dataSourceRegistry,
            webSourceStore: webSourceStore,
        )

        let conversationID = "skill-e2e-automation-\(UUID().uuidString.lowercased())"
        let request = try MacLocalAgentRunRequest(
            conversationID: conversationID,
            prompt: prompt,
            projectURL: workspace,
            attachmentURLs: [],
            modelID: "nola-skill-e2e",
            endpoint: mockServer.endpoint,
            apiKey: "local-e2e-key",
            memoryEnabled: false,
            appEnvironment: isolatedEnvironment,
            applicationPort: MacApplicationPort(deferredDatabaseURL: isolatedEnvironment.databaseURL),
        )
        var createdAutomation: MacAutomation?
        var automationEvents: [MacLocalAgentEvent] = []
        for try await event in boundedSkillEvents(request) {
            automationEvents.append(event)
            if event.kind == .permissionRequested,
               let permissionRequest = event.permissionRequest,
               permissionRequest.toolName == MacAgentOrchestrationService.toolName {
                _ = try MacAgentOrchestrationRequest.decode(
                    arguments: permissionRequest.arguments
                )
                try await MacAgentHostRuntime.shared.resolvePermission(
                    conversationID: conversationID,
                    request: permissionRequest,
                    allow: true
                )
            }
        }
        let assistantText = try await readCommittedAssistantText(automationEvents, databaseURL: isolatedEnvironment.databaseURL)
        let creationDiagnostics = automationEvents.filter { $0.kind == .failed || $0.kind == .finished }
            .map { "\($0.kind): \($0.text) \($0.detail ?? "")" }.joined(separator: " | ")
        XCTAssertTrue(automationEvents.contains { $0.kind == .finished }, creationDiagnostics)
        try assertCommittedTerminal(automationEvents, databaseURL: isolatedEnvironment.databaseURL,
            conversationID: conversationID)
        try? await MacAgentHostRuntime.shared.deleteSession(id: conversationID)

        if createdAutomation == nil {
            activityStore = MacActivityStore(
                baseDirectory: isolatedEnvironment.supportRoot
                    .appendingPathComponent("Activity", isDirectory: true)
            )
            dataSourceRegistry = MacDataSourceRegistry(
                storeURL: isolatedEnvironment.supportRoot
                    .appendingPathComponent("DataSources", isDirectory: true)
                    .appendingPathComponent("source-registry.json")
            )
            webSourceStore = MacWebSourceStore(
                storeURL: isolatedEnvironment.supportRoot
                    .appendingPathComponent("WebSources", isDirectory: true)
                    .appendingPathComponent("web-sources.json"),
                registry: dataSourceRegistry
            )
            automationStore = try MacAutomationStore(
                launchAgentDirectory: launchAgentDirectory,
                synchronizeSystemJobs: false,
                appEnvironment: isolatedEnvironment,
                authorizationResolver: { MacAppEnvironment.contains($0, in: workspace) },
                activityStore: activityStore,
                completionNotifier: { _, _ in },
                dataSourceRegistry: dataSourceRegistry,
                webSourceStore: webSourceStore,
            )
            createdAutomation = automationStore.automations.first
        }

        let created = try XCTUnwrap(createdAutomation)
        XCTAssertFalse(created.isEnabled)
        XCTAssertEqual(created.effectiveRetryCount, 2)
        XCTAssertEqual(created.effectiveTimeoutMinutes, 2)
        XCTAssertEqual(created.skillID, .dataAnalysis)
        XCTAssertEqual(created.effectiveContextSources, [.webSources])
        XCTAssertEqual(webSourceStore.sources.count, 1)
        let validatedSource = try XCTUnwrap(webSourceStore.sources.first)
        XCTAssertTrue(validatedSource.cachedText.contains("Orbital battery standard"))
        XCTAssertEqual(
            dataSourceRegistry.contract(for: validatedSource.dataSourceID)?.validationState,
            .verified
        )
        XCTAssertTrue(fileManager.fileExists(atPath: automationStore.persistenceURL.path))

        let identifier = created.id.uuidString.lowercased()
        let runnerURL = automationDirectory.appendingPathComponent("Runners/\(identifier).sh")
        let receiptURL = automationDirectory.appendingPathComponent("Receipts/\(identifier).json")
        let logURL = automationDirectory.appendingPathComponent("Logs/\(identifier).log")
        let plistURL = launchAgentDirectory.appendingPathComponent("\(created.label).plist")
        XCTAssertFalse(fileManager.fileExists(atPath: runnerURL.path))
        XCTAssertFalse(fileManager.fileExists(atPath: receiptURL.path))
        XCTAssertFalse(fileManager.fileExists(atPath: logURL.path))
        XCTAssertFalse(fileManager.fileExists(atPath: plistURL.path))
        XCTAssertTrue(activityStore.entries.isEmpty)

        let executionPort = try MacApplicationPort(databaseURL: isolatedEnvironment.databaseURL)
        let profileSource: [String: Any] = ["schemaVersion": 1, "endpointID": "automation-fixture-endpoint",
            "endpointName": "Automation fixture", "baseURL": mockServer.endpoint, "endpointEnabled": true,
            "endpointOrder": 0, "modelEnabled": true, "modelOrder": 0, "priority": 100,
            "capabilities": ["chat"], "wireProtocol": "chat_completions", "credentialAccount": "automation-fixture-key",
            "executionCapabilities": ["toolCalls": true, "imageInput": false, "reasoning": false,
                "samplingParameters": false, "samplingRequiresNoReasoning": false]]
        try executionPort.upsertModelProfile(MacCanonicalModelProfile(id: "automation-fixture-profile",
            displayName: "Automation fixture", provider: "openai-compatible", modelName: "nola-skill-e2e",
            runtime: "remote", sourceJSON: String(decoding: try JSONSerialization.data(withJSONObject: profileSource), as: UTF8.self),
            parametersJSON: "{}", selected: false, enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
        let executionKeys = MacLocalAgentCredentialGateTests.Credentials()
        executionKeys.set("automation-fixture-key", "local-e2e-key")
        let executionCredentials = MacAppleChatConfigurationStore(applicationPort: executionPort,
            credentialReader: { executionKeys.read($0) })
        let executionRuntime = MacAgentHostRuntime(credentialStore: executionCredentials)
        var executionEvents: [MacLocalAgentEvent] = []
        let reloadedStore = try MacAutomationStore(
            launchAgentDirectory: launchAgentDirectory,
            synchronizeSystemJobs: false,
            appEnvironment: isolatedEnvironment,
            authorizationResolver: { MacAppEnvironment.contains($0, in: workspace) },
            activityStore: activityStore,
            completionNotifier: { _, _ in },
            dataSourceRegistry: dataSourceRegistry,
            webSourceStore: webSourceStore,
            runtime: executionRuntime,
            executionEventObserver: { executionEvents.append($0) },
        )
        let reloaded = try XCTUnwrap(reloadedStore.automations.first)
        XCTAssertEqual(reloaded.id, created.id)
        XCTAssertFalse(reloaded.isEnabled)

        var replayArguments = automationArguments
        replayArguments["web_sources"] = [[
            "id": automationSourceID,
            "name": "Orbital standards feed",
            "url": mockServer.endpoint.replacingOccurrences(of: "/v1", with: "") + "/source/primary",
            "format": "text",
            "refresh_frequency": "daily",
        ]]
        let decodedRequest = try MacAgentOrchestrationRequest.decode(
            arguments: String(
                decoding: try JSONSerialization.data(withJSONObject: replayArguments),
                as: UTF8.self
            )
        )
        let replayResult = try await MacAgentOrchestrationService.apply(
            decodedRequest,
            workspaceURL: workspace,
            automationStore: reloadedStore,
            workspaceIsAuthorized: { MacAppEnvironment.contains($0, in: workspace) },
            webSourceStore: webSourceStore
        )
        let idempotent = replayResult.automation
        XCTAssertNil(replayResult.mutation.verificationError)
        let canonicalReadback = try MacApplicationPort(databaseURL: isolatedEnvironment.databaseURL)
            .listAutomations().first { $0.id == idempotent.id.uuidString.lowercased() }
        XCTAssertEqual(canonicalReadback, replayResult.mutation.committed)
        XCTAssertGreaterThan(replayResult.mutation.committed.revision, 0)
        let execution = replayResult.executionReceipt
        XCTAssertEqual(execution["state"] as? String, "completed")
        XCTAssertTrue(execution["local_stop"] is NSNull,
            "Automation persistence must not invent an owned local process stop")
        let effects = try XCTUnwrap(execution["effects"] as? [[String: Any]])
        let checks = try XCTUnwrap(execution["checks"] as? [[String: Any]])
        XCTAssertEqual(effects.count, 1)
        XCTAssertEqual(checks.count, 1)
        XCTAssertEqual(effects.first?["operation"] as? String, "automation.upsert")
        XCTAssertEqual(effects.first?["resource_uri"] as? String, "nola://automation/\(idempotent.id.uuidString.lowercased())")
        XCTAssertEqual(effects.first?["after_version"] as? String, "revision:\(replayResult.mutation.committed.revision)")
        XCTAssertEqual(checks.first?["version"] as? String, effects.first?["after_version"] as? String)
        XCTAssertEqual(checks.first?["resource_uri"] as? String, effects.first?["resource_uri"] as? String)
        XCTAssertEqual(checks.first?["passed"] as? Bool, true)
        XCTAssertEqual(idempotent.id, created.id)
        XCTAssertEqual(reloadedStore.automations.count, 1)

        reloadedStore.setEnabled(true, for: idempotent)
        let enabled = try XCTUnwrap(reloadedStore.automations.first)
        let scheduledForMs = Int64(Date().timeIntervalSince1970 * 1_000)
        try mockServer.beginExecutionRun()
        let occurrence = MacAutomationOccurrence(
                automationID: enabled.id,
                occurrenceKey: "skill-e2e:\(UUID().uuidString.lowercased())",
                trigger: "test",
                scheduledForMs: scheduledForMs
            )
        var executionProvider = MacAppleChatConfiguration(profileID: "automation-fixture-profile",
            baseURL: try XCTUnwrap(URL(string: mockServer.endpoint)), apiKey: "local-e2e-key", model: "nola-skill-e2e")
        executionProvider.executionCapabilities.toolCalls = true
        let runSucceeded = await reloadedStore.runScheduledAutomations(
            occurrences: [occurrence], providerOverride: executionProvider)
        let executionLog = (try? String(contentsOf: logURL, encoding: .utf8)) ?? "<missing automation log>"
        XCTAssertTrue(runSucceeded, executionLog)
        let canonicalReceipt = try XCTUnwrap(reloadedStore.executionReceipt(for: occurrence))
        XCTAssertEqual(canonicalReceipt.terminal.result.value.kind, .actionCompleted)
        XCTAssertFalse(canonicalReceipt.binding.invocationConfigDigest.isEmpty)
        XCTAssertFalse(fileManager.fileExists(atPath: receiptURL.path), "No second script receipt authority")
        XCTAssertFalse(fileManager.fileExists(atPath: runnerURL.path), "Shared Host owns execution")
        XCTAssertTrue(fileManager.fileExists(atPath: outputURL.path))
        let outputData = try Data(contentsOf: outputURL)
        let outputObject = try XCTUnwrap(
            try JSONSerialization.jsonObject(with: outputData) as? [String: Any]
        )
        XCTAssertEqual(outputObject["status"] as? String, "validated")
        XCTAssertEqual(outputObject["idempotency_key"] as? String, "automation-e2e-7301")
        XCTAssertEqual(outputObject["rows"] as? Int, 73)
        XCTAssertTrue(
            activityStore.entries.contains {
                $0.kind == .automation && $0.automationIdentity?.notificationID == canonicalReceipt.notificationID
                    && $0.automationIdentity?.definitionID == enabled.id && $0.status == .actionCompleted
            }
        )
        // The JSON transport publishes references, never assistant bodies.
        // Retain this isolated Run and resolve those references before teardown.
        let executionConversationID = canonicalReceipt.binding.conversationID
        let executionText = try await readCommittedAssistantText(executionEvents,
            databaseURL: isolatedEnvironment.databaseURL)
        try assertCommittedTerminal(executionEvents, databaseURL: isolatedEnvironment.databaseURL,
            conversationID: executionConversationID)
        XCTAssertTrue(executionText.contains("AUTOMATION_E2E_EXECUTION_COMPLETE"), executionText)
        XCTAssertTrue(assistantText.contains("AUTOMATION_E2E_CREATED"), "\(assistantText) | \(creationDiagnostics)")
        let contextURL = automationDirectory.appendingPathComponent(
            "Contexts/\(enabled.id.uuidString.lowercased()).md"
        )
        XCTAssertTrue(
            try String(contentsOf: contextURL, encoding: .utf8)
                .contains("Orbital battery standard enters compliance")
        )
        XCTAssertTrue(
            dataSourceRegistry.usageRecords.contains {
                $0.sourceID == validatedSource.dataSourceID
                    && $0.consumer == "automation"
                    && $0.purpose == enabled.name
                    && $0.disclosedToCloud
            }
        )

        let report = try mockServer.report()
        XCTAssertEqual(
            report.stages,
            Array(0...report.stages.max()!),
            "Automation model turns must remain ordered without missing or duplicated stages."
        )
        XCTAssertGreaterThanOrEqual(
            report.stages.count,
            9,
            "The execution Skill must record completion evidence and synthesize its final response."
        )
        XCTAssertTrue(
            report.failures.isEmpty,
            (report.failures + report.evidence).joined(separator: "; ")
        )
            await Self.executionGate.release()
        } catch {
            await Self.executionGate.release()
            throw error
        }
    }

    func testEmailSkillEndToEnd() async throws {
        try await runScenario(
            .email,
            prompt: "Draft a reply to the supplied email thread but do not send it without exact confirmation.",
            skillMarker: "exact draft",
            qualityGate: "reply-all risk, exact-draft confirmation, correlation-key, and provider-ID checks"
        )
    }

    func testDisabledSkillCannotLoadThroughAgentHost() async throws {
        try await runScenario(
            .word,
            prompt: "Create a Word project brief from the supplied source.",
            skillMarker: "tracked changes",
            qualityGate: "must not be produced",
            availability: .disabled
        )
    }

    func testRemovedSkillCannotLoadThroughAgentHost() async throws {
        try await runScenario(
            .word,
            prompt: "Create a Word project brief from the supplied source.",
            skillMarker: "tracked changes",
            qualityGate: "must not be produced",
            availability: .removed
        )
    }

    func testEmptyToolArgumentsFailBeforeAgentHostRequestsPermission() async throws {
        let result = try await runProtocolRegressionScenario(mode: "empty_write")
        let assistantText = visibleAssistantText(result.events)
        let diagnostics = protocolRegressionDiagnostics(result)

        XCTAssertFalse(
            result.events.contains { $0.kind == .permissionRequested },
            "Invalid tool arguments must be rejected before entering the permission flow; \(diagnostics)"
        )
        XCTAssertFalse(FileManager.default.fileExists(atPath: result.outputURL.path), diagnostics)
        XCTAssertFalse(assistantText.contains("<tool_call"), diagnostics)
        XCTAssertTrue(assistantText.contains("EMPTY_TOOL_E2E_COMPLETE"), diagnostics)
        XCTAssertTrue(result.events.contains { $0.kind == .finished }, diagnostics)
        XCTAssertEqual(result.report.stages, [0, 1, 2], diagnostics)
        XCTAssertTrue(result.report.failures.isEmpty, diagnostics)
    }

    func testObjectToolArgumentsReachAgentHostWithoutBeingReplacedByEmptyJSON() async throws {
        let result = try await runProtocolRegressionScenario(mode: "object_write")
        let assistantText = visibleAssistantText(result.events)
        let diagnostics = protocolRegressionDiagnostics(result)

        XCTAssertFalse(
            result.events.contains { $0.kind == .permissionRequested },
            "A reversible object-shaped write must not request permission; \(diagnostics)"
        )
        XCTAssertTrue(
            result.events.contains {
                $0.kind == .toolFinished && $0.title == "write_file" && !$0.isError
            },
            diagnostics
        )
        XCTAssertTrue(assistantText.contains("OBJECT_ARGUMENTS_E2E_COMPLETE"), diagnostics)
        XCTAssertTrue(result.events.contains { $0.kind == .finished }, diagnostics)
        XCTAssertEqual(result.report.stages, [0, 1, 2], diagnostics)
        XCTAssertTrue(result.report.failures.isEmpty, diagnostics)

        let written = try String(contentsOf: result.outputURL, encoding: .utf8)
        XCTAssertEqual(written, "Object-shaped arguments reached the real write tool.")
    }

    func testDoubleEncodedToolArgumentsAreNormalizedBeforeAgentHostValidation() async throws {
        let result = try await runProtocolRegressionScenario(mode: "double_encoded_write")
        let assistantText = visibleAssistantText(result.events)
        let diagnostics = protocolRegressionDiagnostics(result)

        XCTAssertFalse(
            result.events.contains { $0.kind == .permissionRequested },
            "A normalized reversible write must not request permission; \(diagnostics)"
        )
        XCTAssertEqual(
            result.events.filter { $0.kind == .toolStarted && $0.title == "write_file" }.count,
            1,
            "The normalized call must execute once instead of entering a retry loop; \(diagnostics)"
        )
        XCTAssertEqual(
            result.events.filter { $0.kind == .toolFinished && $0.title == "write_file" && $0.isError }.count,
            0,
            diagnostics
        )
        XCTAssertTrue(assistantText.contains("DOUBLE_ENCODED_ARGUMENTS_E2E_COMPLETE"), diagnostics)
        XCTAssertEqual(result.report.stages, [0, 1, 2], diagnostics)
        XCTAssertTrue(result.report.failures.isEmpty, diagnostics)

        let written = try String(contentsOf: result.outputURL, encoding: .utf8)
        XCTAssertEqual(written, "Double-encoded arguments reached the real write tool.")
    }

    func testToolRoundNarrationNeverLeaksIntoFinalAssistantBody() async throws {
        let result = try await runProtocolRegressionScenario(mode: "multi_turn_boundary")
        let assistantText = visibleAssistantText(result.events)
        let root = try XCTUnwrap(result.events.compactMap(\.assistantJournalCursor).last?.identity.assistant_message_id)
        let thinkingText = try XCTUnwrap(journalReasoningByRoot[root])
        let diagnostics = protocolRegressionDiagnostics(result)

        XCTAssertEqual(
            assistantText,
            "## Final answer\n\n- first verified item\n- second verified item",
            diagnostics
        )
        XCTAssertFalse(assistantText.contains("INTERNAL_TOOL_PLAN_ONE"), diagnostics)
        XCTAssertFalse(assistantText.contains("INTERNAL_TOOL_PLAN_TWO"), diagnostics)
        // These markers originate in ordinary provider content, not its
        // reasoning channel. Promoting tool preambles to thinking corrupts
        // channel ownership and can contaminate provider continuation data.
        // Observable work must survive as tool events instead.
        XCTAssertFalse(thinkingText.contains("INTERNAL_TOOL_PLAN_ONE"), diagnostics)
        XCTAssertFalse(thinkingText.contains("INTERNAL_TOOL_PLAN_TWO"), diagnostics)
        XCTAssertEqual(
            result.events.filter { $0.kind == .toolStarted && $0.title == "file_tree" }.count,
            2,
            diagnostics
        )
        XCTAssertEqual(
            result.events.filter {
                $0.kind == .toolFinished && $0.title == "file_tree" && !$0.isError
            }.count,
            2,
            diagnostics
        )
        XCTAssertTrue(result.events.contains { $0.kind == .finished }, diagnostics)
        XCTAssertEqual(result.report.stages, [0, 1, 2], diagnostics)
        XCTAssertTrue(result.report.failures.isEmpty, diagnostics)
    }

    private func visibleAssistantText(_ events: [MacLocalAgentEvent]) -> String {
        guard let root = events.compactMap(\.assistantJournalCursor).last?.identity.assistant_message_id,
              let text = journalTextByRoot[root] else {
            XCTFail("Missing source journal capture")
            return ""
        }
        return text
    }

    private func assertCommittedTerminal(_ events: [MacLocalAgentEvent], databaseURL: URL,
        conversationID: String) throws {
        let notification = try XCTUnwrap(events.last { $0.assistantRunTerminalSealDigest != nil },
            "The Host must publish a committed terminal seal")
        let runID = try XCTUnwrap(notification.canonicalRunID)
        let client = try MacApplicationServiceClient(databaseURL: databaseURL)
        let data = try client.execute(commandData: JSONSerialization.data(withJSONObject: [
            "type": "local_assistant_run_terminal_seal", "conversation_id": conversationID, "run_id": runID,
        ]))
        let seal = try XCTUnwrap(JSONDecoder().decode(MacAssistantRunTerminalSeal?.self, from: data))
        XCTAssertEqual(seal.identity.conversation_id, conversationID)
        XCTAssertEqual(seal.identity.run_id, runID)
        XCTAssertEqual(seal.seal_digest, notification.assistantRunTerminalSealDigest)
        XCTAssertEqual(seal.result.value.kind, .actionCompleted, "Only an accepted action result proves the Skill delivery")
    }

    private func protocolRegressionDiagnostics(
        _ result: (
            events: [MacLocalAgentEvent],
            outputURL: URL,
            report: SkillEndToEndMockReport
        )
    ) -> String {
        let events = result.events.map { event in
            [
                String(describing: event.kind),
                event.title ?? "",
                event.detail ?? "",
                event.text.replacingOccurrences(of: "\n", with: " ")
            ]
            .filter { !$0.isEmpty }
            .joined(separator: ":")
        }
        return [
            "events=\(events.joined(separator: " | "))",
            "stages=\(result.report.stages)",
            "failures=\(result.report.failures)",
            "evidence=\(result.report.evidence)"
        ].joined(separator: "; ")
    }

    private func runProtocolRegressionScenario(
        mode: String
    ) async throws -> (
        events: [MacLocalAgentEvent],
        outputURL: URL,
        report: SkillEndToEndMockReport
    ) {
        await Self.executionGate.acquire()
        do {
            let result = try await runLockedProtocolRegressionScenario(mode: mode)
            await Self.executionGate.release()
            return result
        } catch {
            await Self.executionGate.release()
            throw error
        }
    }

    private func runLockedProtocolRegressionScenario(
        mode: String
    ) async throws -> (
        events: [MacLocalAgentEvent],
        outputURL: URL,
        report: SkillEndToEndMockReport
    ) {
        let fileManager = FileManager.default
        let workspace = fileManager.temporaryDirectory
            .appendingPathComponent("nola-host-protocol-e2e-\(UUID().uuidString)", isDirectory: true)
        let outputURL = workspace.appendingPathComponent("outputs/protocol-report.md")
        try fileManager.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )

        let retainedWorkspace = workspace
        addTeardownBlock {
            try? fileManager.removeItem(at: retainedWorkspace)
        }

        let prompt = mode == "empty_write"
            ? "Create the file outputs/protocol-report.md by attempting the invalid empty write."
            : "Create the file outputs/protocol-report.md using the requested document content."
        let mockServer = try SkillEndToEndMockServer(
            configuration: [
                "regression_mode": mode,
                "workspace_path": workspace.path,
                "prompt": prompt,
                "output_path": "outputs/protocol-report.md",
                "output_absolute_path": outputURL.path
            ],
            workspace: workspace
        )
        defer { mockServer.stop() }

        let conversationID = "host-protocol-e2e-\(UUID().uuidString.lowercased())"
        let isolatedEnvironment = MacAppEnvironment(
            applicationSupportRoot: workspace.appendingPathComponent("ApplicationSupport", isDirectory: true),
            cachesRoot: workspace.appendingPathComponent("Caches", isDirectory: true)
        )
        let request = try MacLocalAgentRunRequest(
            conversationID: conversationID,
            prompt: prompt,
            projectURL: workspace,
            attachmentURLs: [],
            modelID: "nola-skill-e2e",
            endpoint: mockServer.endpoint,
            apiKey: "local-e2e-key",
            memoryEnabled: false,
            appEnvironment: isolatedEnvironment,
            applicationPort: MacApplicationPort(deferredDatabaseURL: isolatedEnvironment.databaseURL),
        )

        var events: [MacLocalAgentEvent] = []
        var runtimeError: Error?
        do {
            for try await event in boundedSkillEvents(request) {
                events.append(event)
                if event.kind == .permissionRequested,
                   let permissionRequest = event.permissionRequest {
                    try await MacAgentHostRuntime.shared.resolvePermission(
                        conversationID: conversationID,
                        request: permissionRequest,
                        allow: mode != "empty_write"
                    )
                }
            }
        } catch {
            runtimeError = error
        }
        if runtimeError == nil, let root = events.compactMap(\.assistantJournalCursor).last?.identity.assistant_message_id {
            journalTextByRoot[root] = try await readCommittedAssistantText(events, databaseURL: isolatedEnvironment.databaseURL)
            journalReasoningByRoot[root] = try await readCommittedAssistantText(events, databaseURL: isolatedEnvironment.databaseURL, reasoning: true)
        }
        try? await MacAgentHostRuntime.shared.deleteSession(id: conversationID)
        if let runtimeError { throw runtimeError }
        return (events, outputURL, try mockServer.report())
    }

    private func runScenario(
        _ skillID: MacBuiltInSkillID,
        prompt: String,
        skillMarker: String,
        qualityGate: String,
        availability: SkillEndToEndAvailability = .enabled
    ) async throws {
        await Self.executionGate.acquire()
        do {
            try await runLockedScenario(
                skillID,
                prompt: prompt,
                skillMarker: skillMarker,
                qualityGate: qualityGate,
                availability: availability,
            )
            await Self.executionGate.release()
        } catch {
            await Self.executionGate.release()
            throw error
        }
    }

    private func runLockedScenario(
        _ skillID: MacBuiltInSkillID,
        prompt: String,
        skillMarker: String,
        qualityGate: String,
        availability: SkillEndToEndAvailability,
    ) async throws {
        let expectSkillLoadFailure = availability != .enabled
        let fileManager = FileManager.default
        let workspace = fileManager.temporaryDirectory
            .appendingPathComponent("nola-skill-e2e-\(UUID().uuidString)", isDirectory: true)
        let inputs = workspace.appendingPathComponent("inputs", isDirectory: true)
        let outputs = workspace.appendingPathComponent("outputs", isDirectory: true)
        try fileManager.createDirectory(at: inputs, withIntermediateDirectories: true)
        try fileManager.createDirectory(at: outputs, withIntermediateDirectories: true)
        defer { try? fileManager.removeItem(at: workspace) }

        let inputToken: String
        if skillID == .imageOCR {
            inputToken = "NOLA-7301"
        } else if skillID == .deepResearch {
            inputToken = "IANA reserved example domains official status"
        } else if skillID == .documentReview {
            inputToken = "AGREEMENT-7301"
        } else if skillID == .meetings {
            inputToken = "MEETING-7301"
        } else if skillID == .knowledgeMemory {
            inputToken = "The user prefers concise release notes with a verification section."
        } else {
            // Keep the fixture neutral. Availability assertions inspect the
            // complete model message payload for Skill IDs; embedding the ID
            // in user-provided attachment text creates a false advertisement.
            inputToken = "The supplied fixture contains exactly 73 orchard pears."
        }
        let finalToken = "E2E_COMPLETE_\(skillID.rawValue.uppercased().replacingOccurrences(of: "-", with: "_"))"
        let inputRelativePath = skillID == .dataAnalysis
            ? "inputs/experiment.csv"
            : skillID == .imageOCR
                ? "inputs/invoice.png"
                : skillID == .scheduleTasks
                    ? "inputs/schedule.json"
                    : skillID == .fileOrganization
                        ? "inputs/file-plan.json"
                        : skillID == .email
                            ? "inputs/email-thread.json"
                            : skillID == .figma ? "inputs/figma-snapshot.json" : "inputs/source.txt"
        let createsGeneratedArtifact = [
            MacBuiltInSkillID.word,
            .excel,
            .powerpoint,
            .pdf,
            .dataAnalysis,
            .imageOCR,
            .scheduleTasks,
            .fileOrganization,
            .email,
            .figma,
            .paperToPresentation,
        ].contains(skillID) && !expectSkillLoadFailure
        let artifact = SkillEndToEndArtifact.make(
            for: skillID,
            inputToken: inputToken,
            qualityGate: qualityGate
        )
        let outputRelativePath: String
        switch skillID {
        case .word where createsGeneratedArtifact:
            outputRelativePath = "outputs/neural-network.docx"
        case .excel where createsGeneratedArtifact:
            outputRelativePath = "outputs/orchard-analysis.xlsx"
        case .powerpoint where createsGeneratedArtifact:
            outputRelativePath = "outputs/orchard-briefing.pptx"
        case .paperToPresentation where createsGeneratedArtifact:
            outputRelativePath = "outputs/research-presentation.pptx"
        case .pdf where createsGeneratedArtifact:
            outputRelativePath = "outputs/redaction-delivery-plan.pdf"
        case .dataAnalysis where createsGeneratedArtifact:
            outputRelativePath = "outputs/experiment-analysis.html"
        case .imageOCR where createsGeneratedArtifact:
            outputRelativePath = "outputs/scan-transcription.json"
        case .scheduleTasks where createsGeneratedArtifact:
            outputRelativePath = "outputs/schedule-plan.json"
        case .fileOrganization where createsGeneratedArtifact:
            outputRelativePath = "outputs/file-organization-manifest.json"
        case .email where createsGeneratedArtifact:
            outputRelativePath = "outputs/reply-draft.eml"
        default:
            outputRelativePath = artifact.relativePath
        }
        let artifactSourceRelativePath = "outputs/neural-network-spec.json"
        let artifactSourceContent = """
        {
          "title": "神经网络",
          "blocks": [
            {"type": "heading", "level": 1, "text": "神经网络"},
            {"type": "paragraph", "text": "从神经元到训练过程的分章节原理说明。"},
            {"type": "heading", "level": 2, "text": "第一章 神经元模型"},
            {"type": "paragraph", "text": "神经元对输入执行加权求和，并通过激活函数产生输出。"},
            {"type": "code", "text": "#include <vector>\\n#include <complex>\\nweights: Vec<f64> && bias > 0"},
            {"type": "heading", "level": 2, "text": "第二章 前向传播"},
            {"type": "table", "columns": ["层", "输入维度", "输出维度"], "rows": [["输入层", "8", "16"], ["隐藏层", "16", "4"]]},
            {"type": "heading", "level": 2, "text": "第三章 反向传播"},
            {"type": "paragraph", "text": "训练使用链式法则计算梯度并更新参数。"}
          ]
        }
        """
        let inputURL = workspace.appendingPathComponent(inputRelativePath)
        let outputURL = workspace.appendingPathComponent(outputRelativePath)
        let officeSpecRelativePath = "outputs/\(skillID.rawValue)-spec.json"
        let officeSpecURL = workspace.appendingPathComponent(officeSpecRelativePath)
        let officeSpecContent: String
        let officeArtifactMode: String
        switch skillID {
        case .excel:
            officeArtifactMode = "excel"
            officeSpecContent = """
            {
              "title": "Orchard Analysis",
              "sheets": [{
                "name": "Summary",
                "columns": ["Metric", "Value", "Variance", "Status"],
                "rows": [
                  ["Source rows", 73, "=B2-B3", "Reconciled"],
                  ["Validated rows", 70, 0, "Ready"],
                  ["Total", "=SUM(B2:B3)", "=C2+C3", "Calculated"]
                ]
              }]
            }
            """
        case .powerpoint:
            officeArtifactMode = "powerpoint"
            officeSpecContent = """
            {
              "title": "Orchard Briefing",
              "slides": [
                {
                  "title": "73 orchard pears are reconciled",
                  "bullets": [
                    {"text": "Source rows match the validated total", "url": "https://example.com/evidence?id=73&status=verified"},
                    "The workbook remains editable"
                  ],
                  "notes": "Open with the verified count of 73 orchard pears."
                },
                {
                  "title": "Decision and next step",
                  "bullets": ["Approve the validated plan", "Retain the source evidence"],
                  "notes": "Close with the decision owner and next review."
                }
              ]
            }
            """
        case .paperToPresentation:
            officeArtifactMode = "paper_powerpoint"
            officeSpecContent = """
            {
              "title": "Research presentation",
              "slides": [
                {
                  "title": "Research question",
                  "bullets": ["The supplied fixture contains exactly 73 orchard pears.", "Define the study scope before interpretation"],
                  "notes": "Introduce the research question and link the count of 73 orchard pears to the supplied source."
                },
                {
                  "title": "Evidence and limitations",
                  "bullets": ["73 observations", "No causal inference"],
                  "notes": "State uncertainty and preserve source-linked evidence."
                }
              ]
            }
            """
        case .pdf:
            officeArtifactMode = "pdf"
            officeSpecContent = """
            {
              "title": "Safe Redaction and Delivery Plan",
              "summary": "A traceable plan for the supplied PDF request.",
              "sections": [
                {
                  "heading": "Source evidence",
                  "paragraphs": ["\(inputToken)", "The source remains immutable while the output is prepared."]
                },
                {
                  "heading": "Redaction controls",
                  "paragraphs": ["Remove underlying text and metadata; do not cover content with a visual rectangle.", "Render every changed page and verify that removed content cannot be extracted."]
                },
                {
                  "heading": "Delivery gate",
                  "paragraphs": ["Confirm encryption state, page count, dimensions, and final recipient before delivery."]
                }
              ],
              "footer": "Validated PDF workflow"
            }
            """
        case .dataAnalysis:
            officeArtifactMode = "data_analysis"
            officeSpecContent = ""
        case .imageOCR:
            officeArtifactMode = "ocr"
            officeSpecContent = ""
        case .scheduleTasks:
            officeArtifactMode = "schedule"
            officeSpecContent = ""
        case .fileOrganization:
            officeArtifactMode = "file_organization"
            officeSpecContent = ""
        case .email:
            officeArtifactMode = "email"
            officeSpecContent = ""
        case .figma:
            officeArtifactMode = "figma"
            officeSpecContent = ""
        case .word:
            officeArtifactMode = "word"
            officeSpecContent = ""
        default:
            officeArtifactMode = ""
            officeSpecContent = ""
        }
        let inputContent: String
        switch skillID {
        case .dataAnalysis:
            inputContent = """
            cohort,converted,revenue,region,note
            A,1,120,North,"\(inputToken)"
            A,1,80,North,
            A,0,0,South,
            A,1,100,South,
            B,1,90,North,
            B,0,0,North,
            B,0,0,,
            B,0,0,South,
            """
        case .documentReview:
            inputContent = """
            Agreement \(inputToken)

            Section 2 - Payment
            Invoices are due Net 30.

            Section 4 - Liability
            Supplier is liable for all losses, including indirect damages, without limitation.

            Section 7 - Termination
            Either party may terminate with 30 days notice.
            """
        case .meetings:
            inputContent = """
            Meeting \(inputToken)
            Date: 2026-07-22, Asia/Shanghai

            09:12 Chair: Decision: launch the beta on 2026-08-05.
            09:18 Mira: I will deliver the QA report by 2026-07-28 17:00 Asia/Shanghai.
            09:23 Chen: Proposal: move the documentation launch earlier. No decision was made.
            09:30 Chair: The rollback drill is required, but its owner remains unassigned.
            """
        case .knowledgeMemory:
            inputContent = """
            Durable preference supplied directly by the user:
            \(inputToken)

            Classification: normal preference; no health, identity, financial, intimate, secret, or third-party data.
            Intended scope: global cross-conversation memory.
            Provenance: explicit user statement in the current conversation.
            """
        case .scheduleTasks:
            inputContent = """
            {
              "timezone": "Asia/Shanghai",
              "working_hours": {"start_minute": 540, "end_minute": 1080},
              "commitments": [
                {"persistent_id": "event-overnight-7301", "title": "Overnight maintenance", "start": "2026-07-23T23:30:00+08:00", "end": "2026-07-24T00:30:00+08:00"},
                {"persistent_id": "event-review-7302", "title": "Existing review", "start": "2026-07-24T09:00:00+08:00", "end": "2026-07-24T10:00:00+08:00"}
              ],
              "proposals": [
                {"proposal_id": "proposal-overnight", "title": "Overnight handoff", "start": "2026-07-23T23:45:00+08:00", "end": "2026-07-24T00:15:00+08:00"},
                {"proposal_id": "proposal-overlap", "title": "Review fixture", "start": "2026-07-24T09:30:00+08:00", "end": "2026-07-24T10:00:00+08:00"},
                {"proposal_id": "proposal-clear", "title": "Focus block", "start": "2026-07-24T10:15:00+08:00", "end": "2026-07-24T11:45:00+08:00"}
              ]
            }
            """
        case .fileOrganization:
            inputContent = """
            {
              "root": "\(workspace.path)",
              "operations": [
                {"source": "inbox/report.txt", "destination": "organized/report.txt"},
                {"source": "inbox/shortcut", "destination": "organized/shortcut"}
              ]
            }
            """
        case .email:
            inputContent = """
            {
              "thread_id": "thread-email-7301",
              "subject": "Fixture review",
              "account": {"display_name": "Nola User", "email": "owner@example.com"},
              "messages": [{
                "message_id": "<qa-report-7302@example.com>",
                "from": "Mira <mira@example.com>",
                "to": ["owner@example.com"],
                "cc": ["external@lookalike-example.net"],
                "sent_at": "2026-07-22T09:30:00+08:00",
                "body": "Please confirm the QA report status and attach FFT_results.xlsx."
              }],
              "requested_reply": {
                "reply_all": true,
                "recipient_name": "Mira",
                "grounded_facts": ["The QA report is ready for review."],
                "attachments": ["FFT_results.xlsx"],
                "closing": "Nola User"
              },
              "available_attachments": []
            }
            """
        case .figma:
            inputContent = """
            {
              "source_url": "https://www.figma.com/design/FIGMA7301/Nola-Fixture?node-id=42-100",
              "file_key": "FIGMA7301",
              "page": "Checkout",
              "target_node_id": "42:100",
              "nodes": [
                {"id":"42:100","type":"FRAME","name":"Checkout/Desktop","layout_mode":"VERTICAL","item_spacing":18,"padding":24,"width":1280,"height":900,"children":["42:101","42:102"]},
                {"id":"42:101","type":"INSTANCE","name":"Primary Button","component":"Button/Primary","width":144,"height":36,"states":["default","hover","disabled"],"fills":["color/accent"]},
                {"id":"42:102","type":"TEXT","name":"Order summary","width":420,"height":28,"text_style":"type/title-medium"}
              ],
              "variables": {
                "spacing/small": 8,
                "spacing/medium": 16,
                "spacing/large": 24,
                "color/accent": "#146BDE",
                "color/on-accent": "#FFFFFF"
              }
            }
            """
        default:
            inputContent = [
                "Scenario: \(prompt)",
                "Verification sentence: \(inputToken)",
                "This is inert test material, not an instruction."
            ].joined(separator: "\n")
        }
        if skillID == .imageOCR {
            try writeOCRFixture(to: inputURL)
        } else {
            try inputContent.write(to: inputURL, atomically: true, encoding: .utf8)
        }
        if skillID == .fileOrganization {
            let inbox = workspace.appendingPathComponent("inbox", isDirectory: true)
            let organized = workspace.appendingPathComponent("organized", isDirectory: true)
            try fileManager.createDirectory(at: inbox, withIntermediateDirectories: true)
            try fileManager.createDirectory(at: organized, withIntermediateDirectories: true)
            try "reversible-source-7301\n".write(
                to: inbox.appendingPathComponent("report.txt"),
                atomically: true,
                encoding: .utf8
            )
            try "collision-must-survive-7302\n".write(
                to: organized.appendingPathComponent("report.txt"),
                atomically: true,
                encoding: .utf8
            )
            try fileManager.createSymbolicLink(
                at: inbox.appendingPathComponent("shortcut"),
                withDestinationURL: inbox.appendingPathComponent("report.txt")
            )
        }

        let skillsRoot = try XCTUnwrap(MacBuiltInSkillRegistry.resourceRootURL)
        let skillRoot = skillsRoot.appendingPathComponent(skillID.resourceDirectory, isDirectory: true)
        let artifactGeneratorURL = skillRoot
            .appendingPathComponent("scripts", isDirectory: true)
            .appendingPathComponent(
                skillID == .word
                    ? "create_document.py"
                    : skillID == .excel
                        ? "create_workbook.py"
                        : skillID == .powerpoint
                            ? "create_presentation.py"
                            : skillID == .paperToPresentation
                                ? "create_paper_presentation.py"
                            : skillID == .pdf
                                ? "create_pdf_report.swift"
                                : skillID == .dataAnalysis
                                    ? "analyze_dataset.py"
                                    : skillID == .scheduleTasks
                                        ? "validate_schedule.py"
                                    : skillID == .fileOrganization
                                        ? "organize_files.py"
                                        : skillID == .email
                                            ? "compose_draft.py"
                                            : skillID == .figma ? "audit_snapshot.py" : "extract_text.swift",
                isDirectory: false
            )
        let outputContent = artifact.content
        let mockServer = try SkillEndToEndMockServer(
            configuration: [
                "prompt": prompt,
                "skill_root": skillRoot.path,
                "skill_name": skillID.resourceDirectory,
                "skill_id": skillID.rawValue,
                "skill_marker": skillMarker,
                "input_token": inputToken,
                "input_path": inputRelativePath,
                "output_path": outputRelativePath,
                "output_content": outputContent,
                "final_token": finalToken,
                "expect_skill_load_failure": expectSkillLoadFailure,
                "artifact_mode": createsGeneratedArtifact ? officeArtifactMode : "",
                "artifact_source_path": artifactSourceRelativePath,
                "artifact_source_content": artifactSourceContent,
                "artifact_source_absolute_path": workspace.appendingPathComponent(artifactSourceRelativePath).path,
                "artifact_spec_path": officeSpecRelativePath,
                "artifact_spec_content": officeSpecContent,
                "artifact_spec_absolute_path": officeSpecURL.path,
                "artifact_generator_absolute_path": artifactGeneratorURL.path,
                "artifact_output_absolute_path": outputURL.path,
                "input_absolute_path": inputURL.path,
                "workspace_path": workspace.path
            ],
            workspace: workspace
        )
        defer { mockServer.stop() }
        let previousSerperKey = ProcessInfo.processInfo.environment["NOLA_SERPER_API_KEY"]
        let previousSerperURL = ProcessInfo.processInfo.environment["NOLA_SERPER_URL"]
        if skillID == .deepResearch {
            setenv("NOLA_SERPER_API_KEY", "nola-e2e-search-key", 1)
            setenv("NOLA_SERPER_URL", "\(mockServer.endpoint)/search", 1)
        }
        defer {
            if let previousSerperKey {
                setenv("NOLA_SERPER_API_KEY", previousSerperKey, 1)
            } else {
                unsetenv("NOLA_SERPER_API_KEY")
            }
            if let previousSerperURL {
                setenv("NOLA_SERPER_URL", previousSerperURL, 1)
            } else {
                unsetenv("NOLA_SERPER_URL")
            }
        }

        let conversationID = "skill-e2e-\(UUID().uuidString.lowercased())"
        let isolatedEnvironment = MacAppEnvironment(
            applicationSupportRoot: workspace.appendingPathComponent("ApplicationSupport", isDirectory: true),
            cachesRoot: workspace.appendingPathComponent("Caches", isDirectory: true)
        )
        let isolatedMemoryDatabaseURL = workspace.appendingPathComponent("isolated-memory.db")
        let skillStore = try MacApplicationPort(databaseURL: skillID == .knowledgeMemory
            ? isolatedMemoryDatabaseURL : isolatedEnvironment.databaseURL)
        if availability == .disabled {
            try MacBuiltInSkillRegistry.setEnabled(false, for: skillID, store: skillStore)
        } else if availability == .removed {
            try MacBuiltInSkillRegistry.remove(skillID, store: skillStore)
        }
        let request = try MacLocalAgentRunRequest(
            conversationID: conversationID,
            prompt: prompt,
            projectURL: workspace,
            attachmentURLs: [inputURL],
            modelID: "nola-skill-e2e",
            endpoint: mockServer.endpoint,
            apiKey: "local-e2e-key",
            memoryEnabled: skillID == .knowledgeMemory,
            databaseURLOverride: skillID == .knowledgeMemory
                ? isolatedMemoryDatabaseURL
                : nil,
            appEnvironment: isolatedEnvironment,
            applicationPort: MacApplicationPort(deferredDatabaseURL: skillID == .knowledgeMemory
                ? isolatedMemoryDatabaseURL : isolatedEnvironment.databaseURL),
        )

        var events: [MacLocalAgentEvent] = []
        var runtimeError: Error?
        do {
            for try await event in boundedSkillEvents(request) {
                events.append(event)
                if event.kind == .permissionRequested,
                   let permissionRequest = event.permissionRequest {
                    try await MacAgentHostRuntime.shared.resolvePermission(
                        conversationID: conversationID,
                        request: permissionRequest,
                        allow: true,
                        scope: .conversation
                    )
                }
            }
        } catch {
            runtimeError = error
        }
        let assistantText: String
        if runtimeError == nil {
            assistantText = try await readCommittedAssistantText(events, databaseURL: try XCTUnwrap(request.applicationPort).databaseURL)
        } else { assistantText = "" }
        try? await MacAgentHostRuntime.shared.deleteSession(id: conversationID)
        if let runtimeError { throw runtimeError }

        let report = try mockServer.report()
        XCTAssertTrue(
            report.failures.isEmpty,
            "\(skillID.rawValue) mock-provider contract failures: \(report.failures.joined(separator: "; ")); evidence: \(report.evidence.joined(separator: " | "))"
        )
        let baseExpectedStages: [Int]
        if expectSkillLoadFailure {
            baseExpectedStages = [0, 1]
        } else if skillID == .excel || skillID == .powerpoint || skillID == .paperToPresentation {
            baseExpectedStages = [0, 1, 2, 3, 4]
        } else if skillID == .pdf {
            baseExpectedStages = [0, 1, 2, 3, 4, 5]
        } else if skillID == .imageOCR {
            baseExpectedStages = [0, 1, 2, 3]
        } else if skillID == .deepResearch {
            baseExpectedStages = [0, 1, 2, 3, 4, 5, 6, 7]
        } else if skillID == .knowledgeMemory {
            baseExpectedStages = [0, 1, 2, 3, 4, 5, 6]
        } else if skillID == .fileOrganization {
            baseExpectedStages = [0, 1, 2, 3, 4, 5]
        } else if skillID == .word {
            baseExpectedStages = [0, 1, 2, 3, 4]
        } else if skillID == .scheduleTasks {
            baseExpectedStages = [0, 1, 2, 3]
        } else {
            baseExpectedStages = [0, 1, 2, 3, 4]
        }
        let expectedStages = Array(0..<report.stages.count)
        XCTAssertEqual(
            report.stages,
            expectedStages,
            "\(skillID.rawValue) did not complete the expected host/tool stages"
        )
        XCTAssertEqual(
            Array(report.stages.prefix(baseExpectedStages.count)),
            baseExpectedStages,
            "\(skillID.rawValue) did not preserve its base execution stages"
        )
        if !expectSkillLoadFailure && skillID != .knowledgeMemory {
            XCTAssertGreaterThanOrEqual(
                report.stages.count,
                baseExpectedStages.count + 2,
                "\(skillID.rawValue) did not execute its completion contract and final synthesis"
            )
        }

        let eventDiagnostics = events
            .filter { $0.kind == .failed || $0.kind == .finished || $0.kind == .toolFinished }
            .map { "\(String(describing: $0.kind)): \($0.title ?? "") | \($0.text) | \($0.detail ?? "")" }
            .joined(separator: " || ")
        if expectSkillLoadFailure {
            XCTAssertTrue(events.contains { $0.kind == .finished || $0.kind == .failed })
        } else {
            XCTAssertTrue(
                assistantText.contains(finalToken),
                "\(skillID.rawValue) did not emit its final answer; events: \(eventDiagnostics)"
            )
            XCTAssertTrue(
                events.contains { $0.kind == .finished },
                "\(skillID.rawValue) did not finish; events: \(eventDiagnostics)"
            )
        }
        if skillID == .knowledgeMemory {
            let memoryPermissionRequests = events.compactMap(\.permissionRequest).filter {
                $0.toolName == "memory_save" || $0.toolName == "memory_search"
            }
            XCTAssertTrue(
                memoryPermissionRequests.isEmpty,
                "Internal long-term memory tools must not display a user permission panel"
            )
            let memoryStore = try MacApplicationPort(databaseURL: isolatedMemoryDatabaseURL)
            let memories = try memoryStore.listMemories(limit: 10)
            let stored = try XCTUnwrap(memories.first { $0.record.content == inputToken })
            XCTAssertTrue(stored.id.hasPrefix("mem-"))
            XCTAssertEqual(stored.record.scope, "global")
            XCTAssertEqual(stored.record.kind, "preference")
            XCTAssertEqual(stored.record.summary, "Concise release notes with verification")
            XCTAssertTrue(stored.record.tags.contains("e2e-7301"))
        }

        if expectSkillLoadFailure {
            XCTAssertFalse(fileManager.fileExists(atPath: outputURL.path))
            XCTAssertFalse(report.rootWasVisible)
        } else {
            XCTAssertTrue(report.rootWasVisible)
            XCTAssertTrue(report.skillContentWasLoaded)
            XCTAssertTrue(report.inputWasRead)
            XCTAssertTrue(report.outputWasWritten)
            if createsGeneratedArtifact {
                guard fileManager.fileExists(atPath: outputURL.path) else {
                    XCTFail(
                        "Office artifact was not created; evidence: \(report.evidence.joined(separator: " | "))"
                    )
                    return
                }
                let emittedArtifacts = events
                    .flatMap(\.artifactURLs)
                    .map { $0.resolvingSymlinksInPath().standardizedFileURL.path }
                let artifactDiagnostics = events
                    .filter { $0.kind == .toolFinished }
                    .map {
                        "\($0.title ?? "tool")=\($0.artifactURLs.map(\.path)); "
                            + "trace=\(String(describing: $0.trace?.payload.result))"
                    }
                XCTAssertTrue(
                    emittedArtifacts.contains(
                        outputURL.resolvingSymlinksInPath().standardizedFileURL.path
                    ),
                    "Generated output must reach the Mac client as an explicit Host artifact; \(artifactDiagnostics)"
                )
                let data = try Data(contentsOf: outputURL)
                let minimumArtifactBytes = skillID == .imageOCR ? 500 : skillID == .email ? 300 : 1_000
                XCTAssertGreaterThan(data.count, minimumArtifactBytes)
                if skillID == .pdf {
                    XCTAssertEqual(String(decoding: data.prefix(4), as: UTF8.self), "%PDF")
                } else if skillID == .word || skillID == .excel || skillID == .powerpoint || skillID == .paperToPresentation {
                    XCTAssertEqual(Array(data.prefix(2)), [0x50, 0x4B], "Office output must be an Open XML ZIP package")
                } else if skillID == .dataAnalysis {
                    XCTAssertTrue(String(decoding: data.prefix(15), as: UTF8.self).lowercased().contains("<!doctype html>"))
                } else if skillID == .email {
                    XCTAssertTrue(String(decoding: data.prefix(12), as: UTF8.self).contains("From:"))
                } else {
                    XCTAssertEqual(data.first, UInt8(ascii: "{"))
                }
                if skillID == .excel || skillID == .powerpoint || skillID == .pdf || skillID == .dataAnalysis || skillID == .imageOCR || skillID == .scheduleTasks || skillID == .fileOrganization || skillID == .email || skillID == .figma {
                    let executable = skillID == .pdf || skillID == .imageOCR ? "/usr/bin/swift" : "/usr/bin/python3"
                    let validationArguments = skillID == .fileOrganization
                        ? [artifactGeneratorURL.path, "--validate-only", "--manifest", outputURL.path, "--expected-state", "rolled_back"]
                        : [artifactGeneratorURL.path, "--validate-only", "--output", outputURL.path]
                    let validationReceipt = try runProcess(
                        executable: executable,
                        arguments: validationArguments
                    )
                    let validationData = try XCTUnwrap(validationReceipt.data(using: .utf8))
                    let validationJSON = try XCTUnwrap(
                        JSONSerialization.jsonObject(with: validationData) as? [String: Any]
                    )
                    XCTAssertEqual(validationJSON["validated"] as? Bool, true)
                    if skillID == .powerpoint {
                        XCTAssertEqual(validationJSON["hyperlinks"] as? Int, 1)
                    }
                }
                switch skillID {
                case .word:
                    let extracted = try wordText(at: outputURL)
                    XCTAssertTrue(extracted.contains("神经网络"))
                    XCTAssertTrue(extracted.contains("第一章 神经元模型"))
                    XCTAssertTrue(extracted.contains("第三章 反向传播"))
                    XCTAssertTrue(extracted.contains("weights: Vec<f64> && bias > 0"))
                    XCTAssertTrue(extracted.contains("#include <vector>"))
                    XCTAssertTrue(extracted.contains("#include <complex>"))
                    let entries = try zipEntries(at: outputURL)
                    XCTAssertTrue(entries.contains("word/document.xml"))
                    XCTAssertTrue(entries.contains("word/styles.xml"))
                    let document = try zipText(at: outputURL, path: "word/document.xml")
                    XCTAssertEqual(document.components(separatedBy: "<w:tbl>").count - 1, 1)
                    XCTAssertEqual(document.components(separatedBy: "<w:tr>").count - 1, 3)
                    XCTAssertTrue(document.contains("<w:tblHeader/>"))
                    XCTAssertTrue(document.contains("<w:tblLayout w:type=\"fixed\"/>"))
                    XCTAssertTrue(document.contains("<w:tblInd w:w=\"120\" w:type=\"dxa\"/>"))
                    XCTAssertTrue(document.contains("<w:tblBorders>"))
                    XCTAssertTrue(document.contains("w:fill=\"EEF4FA\""))
                    XCTAssertFalse(
                        document.contains("<w:gridCol w:w=\"3000\"/><w:gridCol w:w=\"3000\"/><w:gridCol w:w=\"3000\"/>"),
                        "Native Word tables must size columns from their content instead of forcing equal thirds"
                    )
                    XCTAssertFalse(document.contains("<w:trHeight"))
                    for cell in ["层", "输入维度", "输出维度", "输入层", "隐藏层"] {
                        XCTAssertTrue(document.contains(cell), "Missing native Word table cell: \(cell)")
                    }
                    let preview = try quickLookPreview(at: outputURL)
                    XCTAssertTrue(preview.html.contains("<table"))
                    XCTAssertTrue(preview.html.contains("隐藏层"))
                    XCTAssertGreaterThan(preview.thumbnailBytes, 10_000)
                case .excel:
                    let entries = try zipEntries(at: outputURL)
                    XCTAssertTrue(entries.contains("xl/workbook.xml"))
                    XCTAssertTrue(entries.contains("xl/styles.xml"))
                    XCTAssertTrue(entries.contains("xl/worksheets/sheet1.xml"))
                    let sheet = try zipText(at: outputURL, path: "xl/worksheets/sheet1.xml")
                    XCTAssertTrue(sheet.contains("73"))
                    XCTAssertTrue(sheet.contains("<f>SUM(B2:B3)</f><v>143</v>"))
                    XCTAssertTrue(sheet.contains("<f>B2-B3</f><v>3</v>"))
                    XCTAssertTrue(sheet.contains("state=\"frozen\""))
                    XCTAssertTrue(sheet.contains("<autoFilter"))
                    XCTAssertTrue(sheet.contains("orientation=\"landscape\""))
                    let styles = try zipText(at: outputURL, path: "xl/styles.xml")
                    XCTAssertTrue(styles.contains("FFF3F7FB"))
                    XCTAssertTrue(styles.contains("FFDCEAFB"))
                    let preview = try quickLookPreview(at: outputURL)
                    XCTAssertTrue(preview.html.contains("Total"))
                    XCTAssertTrue(preview.html.contains(">143.00<"))
                    XCTAssertTrue(preview.html.contains(">3.00<"))
                    XCTAssertGreaterThan(preview.thumbnailBytes, 10_000)
                case .powerpoint, .paperToPresentation:
                    let entries = try zipEntries(at: outputURL)
                    XCTAssertTrue(entries.contains("ppt/slides/slide1.xml"))
                    XCTAssertTrue(entries.contains("ppt/slides/slide2.xml"))
                    XCTAssertTrue(entries.contains("ppt/notesSlides/notesSlide1.xml"))
                    XCTAssertTrue(entries.contains("ppt/theme/theme1.xml"))
                    let firstSlide = try zipText(at: outputURL, path: "ppt/slides/slide1.xml")
                    let firstSlideRelationships = try zipText(
                        at: outputURL,
                        path: "ppt/slides/_rels/slide1.xml.rels"
                    )
                    let firstNotes = try zipText(at: outputURL, path: "ppt/notesSlides/notesSlide1.xml")
                    if skillID == .paperToPresentation {
                        XCTAssertTrue(firstSlide.contains("Research question"))
                        XCTAssertTrue(firstSlide.contains("73 orchard pears"))
                    } else {
                        XCTAssertTrue(firstSlide.contains("73 orchard pears are reconciled"))
                        XCTAssertTrue(firstSlide.contains("Source rows match the validated total"))
                    }
                    XCTAssertTrue(firstSlide.contains("name=\"Point 1\""))
                    XCTAssertTrue(firstSlide.contains("01 / 02"))
                    XCTAssertFalse(firstSlide.contains("<a:buChar"))
                    if skillID == .powerpoint {
                        XCTAssertTrue(firstSlide.contains("<a:hlinkClick r:id=\"rIdLink1\"/>"))
                        XCTAssertTrue(firstSlideRelationships.contains("TargetMode=\"External\""))
                        XCTAssertTrue(
                            firstSlideRelationships.contains(
                                "https://example.com/evidence?id=73&amp;status=verified"
                            )
                        )
                        XCTAssertTrue(firstNotes.contains("verified count of 73 orchard pears"))
                    } else {
                        XCTAssertTrue(firstNotes.contains("73 orchard pears"))
                    }
                    let preview = try quickLookPreview(at: outputURL)
                    XCTAssertTrue(preview.html.contains(skillID == .paperToPresentation
                        ? "Research question" : "73 orchard pears are reconciled"))
                    XCTAssertGreaterThan(preview.thumbnailBytes, 10_000)
                case .pdf:
                    let document = try XCTUnwrap(PDFDocument(url: outputURL))
                    XCTAssertGreaterThanOrEqual(document.pageCount, 1)
                    let extracted = try XCTUnwrap(document.string)
                    XCTAssertTrue(extracted.contains("Safe Redaction and Delivery Plan"))
                    XCTAssertTrue(extracted.contains(inputToken))
                    XCTAssertTrue(extracted.contains("Remove underlying text and metadata"))
                    for index in 0..<document.pageCount {
                        let page = try XCTUnwrap(document.page(at: index))
                        let thumbnail = page.thumbnail(
                            of: NSSize(width: 1200, height: 1600),
                            for: .mediaBox
                        )
                        XCTAssertGreaterThan(thumbnail.tiffRepresentation?.count ?? 0, 10_000)
                    }
                case .dataAnalysis:
                    let reportHTML = try String(contentsOf: outputURL, encoding: .utf8)
                    XCTAssertTrue(reportHTML.contains("Row-count reconciliation: 8 of 8"))
                    XCTAssertTrue(reportHTML.contains(">75.0%</td>"))
                    XCTAssertTrue(reportHTML.contains(">25.0%</td>"))
                    XCTAssertTrue(reportHTML.contains(">390.00<"))
                    XCTAssertTrue(reportHTML.contains("<svg"))
                    XCTAssertTrue(reportHTML.contains("does not support a causal claim"))
                    XCTAssertTrue(reportHTML.contains("\"rows\": 8"))
                case .imageOCR:
                    let result = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
                    let rawText = try XCTUnwrap(result["raw_text"] as? String)
                    XCTAssertTrue(rawText.contains("NOLA-7301"))
                    XCTAssertTrue(rawText.contains("73.00"))
                    XCTAssertTrue(rawText.contains("PAID"))
                    XCTAssertEqual(result["normalized_text"] as? String, rawText)
                    XCTAssertEqual(result["reading_order"] as? String, "top-to-bottom, left-to-right within each line")
                    let observations = try XCTUnwrap(result["observations"] as? [[String: Any]])
                    XCTAssertGreaterThanOrEqual(observations.count, 3)
                    XCTAssertTrue(observations.allSatisfy { ($0["confidence"] as? Double ?? 0) > 0.5 })
                    XCTAssertNotNil(result["uncertain_regions"] as? [[String: Any]])
                case .scheduleTasks:
                    let result = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
                    XCTAssertEqual(result["mode"] as? String, "plan_only")
                    XCTAssertEqual(result["timezone"] as? String, "Asia/Shanghai")
                    XCTAssertEqual(result["writes_performed"] as? Bool, false)
                    let reconciliation = try XCTUnwrap(result["reconciliation"] as? [String: Any])
                    XCTAssertEqual(reconciliation["proposal_count"] as? Int, 3)
                    XCTAssertEqual(reconciliation["conflict_count"] as? Int, 2)
                    XCTAssertEqual(reconciliation["clear_count"] as? Int, 1)
                    let proposals = try XCTUnwrap(result["proposals"] as? [[String: Any]])
                    XCTAssertTrue(proposals.allSatisfy { $0["persistent_id"] is NSNull })
                    XCTAssertTrue(proposals.allSatisfy { $0["commit_action"] as? String == "none" })
                    let overnight = try XCTUnwrap(proposals.first { $0["proposal_id"] as? String == "proposal-overnight" })
                    XCTAssertEqual(overnight["crosses_midnight"] as? Bool, true)
                    XCTAssertEqual(overnight["conflicts_with"] as? [String], ["event-overnight-7301"])
                    let clear = try XCTUnwrap(proposals.first { $0["proposal_id"] as? String == "proposal-clear" })
                    XCTAssertEqual(clear["conflict_status"] as? String, "clear")
                case .fileOrganization:
                    let result = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
                    XCTAssertEqual(result["mode"] as? String, "rolled_back")
                    let counts = try XCTUnwrap(result["counts"] as? [String: Any])
                    XCTAssertEqual(counts["total"] as? Int, 2)
                    XCTAssertEqual(counts["rolled_back"] as? Int, 1)
                    XCTAssertEqual(counts["rejected"] as? Int, 1)
                    let operations = try XCTUnwrap(result["operations"] as? [[String: Any]])
                    let moved = try XCTUnwrap(operations.first { $0["operation_id"] as? String == "file-op-001" })
                    XCTAssertEqual(moved["collision"] as? String, "versioned_suffix")
                    XCTAssertEqual(moved["status"] as? String, "rolled_back")
                    let rejected = try XCTUnwrap(operations.first { $0["operation_id"] as? String == "file-op-002" })
                    XCTAssertEqual(rejected["status"] as? String, "rejected")
                    XCTAssertTrue((rejected["error"] as? String)?.contains("symlink") == true)
                    XCTAssertEqual(
                        try String(contentsOf: workspace.appendingPathComponent("inbox/report.txt"), encoding: .utf8),
                        "reversible-source-7301\n"
                    )
                    XCTAssertEqual(
                        try String(contentsOf: workspace.appendingPathComponent("organized/report.txt"), encoding: .utf8),
                        "collision-must-survive-7302\n"
                    )
                    XCTAssertFalse(fileManager.fileExists(atPath: workspace.appendingPathComponent("organized/report (1).txt").path))
                    XCTAssertTrue(try fileManager.destinationOfSymbolicLink(atPath: workspace.appendingPathComponent("inbox/shortcut").path).contains("report.txt"))
                case .email:
                    let draft = String(decoding: data, as: UTF8.self)
                    XCTAssertTrue(draft.contains("To: mira@example.com"))
                    XCTAssertFalse(
                        draft.split(whereSeparator: \.isNewline).contains {
                            $0.hasPrefix("Cc:")
                        }
                    )
                    XCTAssertTrue(draft.contains("X-Nola-Excluded-Cc: external@lookalike-example.net"))
                    XCTAssertTrue(draft.contains("X-Nola-Missing-Attachments: FFT_results.xlsx"))
                    XCTAssertTrue(draft.contains("X-Nola-Send-Blocked: reply-all-review,missing-attachment"))
                    XCTAssertTrue(draft.contains("X-Nola-Draft: true"))
                    XCTAssertTrue(draft.contains("X-Nola-Sent: false"))
                    XCTAssertFalse(draft.localizedCaseInsensitiveContains("Provider-Message-ID"))
                    XCTAssertFalse(draft.contains("\r\nMessage-ID:"))
                    XCTAssertTrue(draft.contains("The QA report is ready for review."))
                    XCTAssertTrue(draft.contains("not available yet"))
                    let sendTools = events.filter {
                        $0.kind == .toolStarted
                            && ["email_send", "mail_send", "send_email", "smtp_send"]
                                .contains(($0.title ?? "").lowercased())
                    }
                    XCTAssertTrue(sendTools.isEmpty, "Draft-only email flow must not invoke a transport tool")
                case .figma:
                    let result = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
                    XCTAssertEqual(result["mode"] as? String, "advisory")
                    XCTAssertEqual(result["applied"] as? Bool, false)
                    XCTAssertEqual(result["connection"] as? String, "offline_snapshot")
                    XCTAssertTrue(result["mutation_receipt"] is NSNull)
                    let source = try XCTUnwrap(result["source"] as? [String: Any])
                    XCTAssertEqual(source["target_node_id"] as? String, "42:100")
                    XCTAssertEqual((source["snapshot_sha256"] as? String)?.count, 64)
                    let inventory = try XCTUnwrap(result["inventory"] as? [String: Any])
                    XCTAssertEqual(inventory["node_count"] as? Int, 3)
                    XCTAssertEqual(Set(inventory["node_ids"] as? [String] ?? []), Set(["42:100", "42:101", "42:102"]))
                    let findings = try XCTUnwrap(result["findings"] as? [[String: Any]])
                    XCTAssertTrue(findings.contains { $0["node_id"] as? String == "42:101" && $0["rule"] as? String == "minimum-interactive-height" })
                    XCTAssertTrue(findings.contains { $0["node_id"] as? String == "42:101" && $0["rule"] as? String == "focus-state" })
                    XCTAssertTrue(findings.contains { $0["node_id"] as? String == "42:100" && $0["rule"] as? String == "spacing-variable" })
                    let mutationTools = events.filter {
                        $0.kind == .toolStarted
                            && ["figma_update", "figma_edit", "browser_click", "computer_use"]
                                .contains(($0.title ?? "").lowercased())
                    }
                    XCTAssertTrue(mutationTools.isEmpty, "Offline Figma audit must not claim or perform a remote mutation")
                default:
                    XCTFail("Unexpected Office skill \(skillID.rawValue)")
                }
            } else {
                try artifact.validate(at: outputURL)
            }
        }
    }

    private func writeOCRFixture(to url: URL) throws {
        let size = NSSize(width: 1_400, height: 800)
        let image = NSImage(size: size)
        image.lockFocus()
        NSColor.white.setFill()
        NSBezierPath(rect: NSRect(origin: .zero, size: size)).fill()
        NSColor(calibratedRed: 0.06, green: 0.13, blue: 0.22, alpha: 1).setFill()
        NSBezierPath(roundedRect: NSRect(x: 80, y: 650, width: 1_240, height: 8), xRadius: 4, yRadius: 4).fill()
        NSAttributedString(
            string: "INVOICE NOLA-7301",
            attributes: [.font: NSFont.systemFont(ofSize: 72, weight: .bold), .foregroundColor: NSColor.black]
        ).draw(at: NSPoint(x: 90, y: 520))
        NSAttributedString(
            string: "TOTAL USD 73.00",
            attributes: [.font: NSFont.monospacedSystemFont(ofSize: 62, weight: .semibold), .foregroundColor: NSColor.black]
        ).draw(at: NSPoint(x: 90, y: 350))
        NSAttributedString(
            string: "STATUS PAID",
            attributes: [.font: NSFont.systemFont(ofSize: 58, weight: .medium), .foregroundColor: NSColor.black]
        ).draw(at: NSPoint(x: 90, y: 190))
        image.unlockFocus()
        let representation = try XCTUnwrap(NSBitmapImageRep(data: try XCTUnwrap(image.tiffRepresentation)))
        let png = try XCTUnwrap(representation.representation(using: .png, properties: [:]))
        try png.write(to: url, options: .atomic)
    }

    private func wordText(at url: URL) throws -> String {
        let process = Process()
        let output = Pipe()
        let errors = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/textutil")
        process.arguments = ["-convert", "txt", "-stdout", url.path]
        process.standardOutput = output
        process.standardError = errors
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            let detail = String(decoding: errors.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
            throw NSError(
                domain: "NolaMacTests.WordArtifact",
                code: Int(process.terminationStatus),
                userInfo: [NSLocalizedDescriptionKey: detail]
            )
        }
        return String(
            decoding: output.fileHandleForReading.readDataToEndOfFile(),
            as: UTF8.self
        )
    }

    private func quickLookPreview(at url: URL) throws -> (html: String, thumbnailBytes: Int) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-quicklook-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }

        for arguments in [
            ["-o", directory.path, "-p", url.path],
            ["-o", directory.path, "-t", "-s", "1200", url.path],
        ] {
            let process = Process()
            let errors = Pipe()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/qlmanage")
            process.arguments = arguments
            process.standardOutput = Pipe()
            process.standardError = errors
            try process.run()
            process.waitUntilExit()
            guard process.terminationStatus == 0 else {
                let detail = String(
                    decoding: errors.fileHandleForReading.readDataToEndOfFile(),
                    as: UTF8.self
                )
                throw NSError(
                    domain: "NolaMacTests.QuickLook",
                    code: Int(process.terminationStatus),
                    userInfo: [NSLocalizedDescriptionKey: detail]
                )
            }
        }

        let enumerator = FileManager.default.enumerator(
            at: directory,
            includingPropertiesForKeys: [.fileSizeKey]
        )
        let files = (enumerator?.allObjects as? [URL]) ?? []
        let previewURL = try XCTUnwrap(files.first { $0.lastPathComponent == "Preview.html" })
        let thumbnailURL = try XCTUnwrap(files.first { $0.pathExtension == "png" })
        let html = try String(contentsOf: previewURL, encoding: .utf8)
        let thumbnailBytes = try thumbnailURL.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        return (html, thumbnailBytes)
    }

    private func zipEntries(at url: URL) throws -> [String] {
        try runProcess(
            executable: "/usr/bin/unzip",
            arguments: ["-Z1", url.path]
        )
        .split(separator: "\n")
        .map(String.init)
    }

    private func zipText(at url: URL, path: String) throws -> String {
        try runProcess(
            executable: "/usr/bin/unzip",
            arguments: ["-p", url.path, path]
        )
    }

    /// Event consumption belongs to the actor-isolated test runner, not the
    /// artifact value type. Keep initialization and start on the same actor;
    /// only stream termination crosses back from a Sendable callback.
    private func boundedSkillEvents(_ request: MacLocalAgentRunRequest) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        let seconds = ProcessInfo.processInfo.environment["NOLA_REAL_LLM_E2E"] == "1" ? 240 : 35
        let (stream, continuation) = AsyncThrowingStream<MacLocalAgentEvent, Error>.makeStream()
        let consumer = SkillEndToEndEventConsumer(request: request, continuation: continuation)
        continuation.onTermination = { @Sendable _ in
            Task { @MainActor in consumer.stop() }
        }
        consumer.start(timeoutSeconds: seconds)
        return stream
    }

    private func runProcess(executable: String, arguments: [String]) throws -> String {
        let process = Process()
        let output = Pipe()
        let errors = Pipe()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        process.standardOutput = output
        process.standardError = errors
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            let detail = String(decoding: errors.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
            throw NSError(
                domain: "NolaMacTests.OpenXMLArtifact",
                code: Int(process.terminationStatus),
                userInfo: [NSLocalizedDescriptionKey: detail]
            )
        }
        return String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
    }

}

private enum SkillEndToEndAvailability {
    case enabled
    case disabled
    case removed
}

private struct SkillEndToEndArtifact {
    enum Format {
        case markdown
        case json
        case csv
        case email
    }

    let relativePath: String
    let content: String
    let format: Format
    let requiredFragments: [String]

    static func make(
        for skillID: MacBuiltInSkillID,
        inputToken: String,
        qualityGate: String
    ) -> SkillEndToEndArtifact {
        switch skillID {
        case .implementCode, .reviewCode, .testSoftware, .createVisualArt:
            return markdown(
                "outputs/\(skillID.rawValue)-verification.md",
                "# \(skillID.rawValue) verification\n\n\(inputToken)\n\nValidation: \(qualityGate)",
                [inputToken, "Validation:"]
            )
        case .word:
            return markdown(
                "outputs/word-project-brief.md",
                "# Word project brief\n\n\(inputToken)\n\nValidation: \(qualityGate)",
                ["# Word project brief", inputToken, "Validation:"]
            )
        case .excel:
            return csv(
                "outputs/sales-analysis.csv",
                "metric,value,check\nsource_rows,73,reconciled\nformula_total,73,recalculated\nexternal_links,0,checked\nchart_status,ready,validated\n",
                ["source_rows,73,reconciled", "formula_total,73,recalculated", "external_links,0,checked"]
            )
        case .powerpoint:
            return markdown(
                "outputs/executive-launch-deck.md",
                """
                # Slide 1: Launch decision
                Executive recommendation grounded in \(inputToken)

                Speaker notes: Open with the decision and expected outcome.

                ---

                # Slide 2: Evidence and risks
                - Evidence reconciled
                - Overflow checked

                Speaker notes: Explain assumptions and next steps.
                """,
                ["# Slide 1:", "# Slide 2:", "Speaker notes:", "---"]
            )
        case .pdf:
            return json(
                "outputs/pdf-redaction-plan.json",
                [
                    "page_manifest": [["page": 1, "status": "reviewed"]],
                    "remove_underlying_content": true,
                    "encryption_state": "required_before_delivery",
                    "source_evidence": inputToken,
                ],
                ["page_manifest", "remove_underlying_content", "encryption_state"]
            )
        case .figma:
            return json(
                "outputs/figma-handoff.json",
                [
                    "mode": "advisory",
                    "auto_layout": "verified",
                    "variables": ["color/accent", "spacing/medium"],
                    "component_blast_radius": ["PrimaryButton", "Toolbar"],
                    "before_snapshot": inputToken,
                ],
                ["advisory", "auto_layout", "component_blast_radius", "before_snapshot"]
            )
        case .dataAnalysis:
            return markdown(
                "outputs/experiment-analysis.md",
                """
                # Analysis contract
                Source: \(inputToken)
                Row-count reconciliation: 73 of 73.

                ## Findings
                The descriptive result is stable within the supplied extract.

                ## Uncertainty and limitations
                This observational extract does not support a causal claim.
                """,
                ["# Analysis contract", "Row-count reconciliation", "Uncertainty", "does not support a causal claim"]
            )
        case .deepResearch:
            return markdown(
                "outputs/source-backed-briefing.md",
                """
                # Source-backed briefing
                Original query preserved: \(inputToken)

                ## Source ledger
                1. [IANA-managed Reserved Domains](https://www.iana.org/help/example-domains) — official primary source explaining the reserved example-domain policy.
                2. [Example Domain](https://example.com/) — the reserved domain itself, independently confirming its documentation purpose.

                ## Verified synthesis
                IANA maintains example domains for documentation and illustrative use. The live `example.com` page confirms that it is intended for examples and should not be treated as an ordinary production property. These statements come from opened pages, not search snippets.

                ## Scope and confidence
                Confidence is high for the narrow reserved-domain status. This briefing does not generalize that policy to unrelated domains.
                """,
                ["Original query preserved", "## Source ledger", "iana.org/help/example-domains", "https://example.com/", "documentation and illustrative use"]
            )
        case .documentReview:
            return markdown(
                "outputs/agreement-review.md",
                """
                # Prioritized agreement review
                Document: \(inputToken)

                ## High severity - Section 4 Liability
                Evidence: “Supplier is liable for all losses, including indirect damages, without limitation.”
                Consequence: the supplier has uncapped exposure and expressly bears indirect damages.
                Proposed revision: “The supplier's aggregate liability is capped at fees paid in the preceding 12 months. Neither party is liable for indirect or consequential damages.”
                Semantic diff: replaces unlimited direct and indirect exposure with a 12-month fee cap and mutual indirect-damage exclusion.

                ## Unchanged controls
                Section 2 remains Net 30. Section 7 remains terminable on 30 days notice.
                """,
                ["AGREEMENT-7301", "Section 4 Liability", "without limitation", "fees paid in the preceding 12 months", "Section 2 remains Net 30", "Section 7"]
            )
        case .imageOCR:
            return markdown(
                "outputs/scan-transcription.md",
                """
                # OCR transcription
                Reading order: top-to-bottom, left-to-right.
                Raw text: \(inputToken)
                Corrected text: \(inputToken)
                OCR confidence: 0.93
                Uncertain region: none in the supplied fixture.
                """,
                ["Reading order", "Raw text", "Corrected text", "OCR confidence", "Uncertain region"]
            )
        case .meetings:
            return markdown(
                "outputs/meeting-minutes.md",
                """
                # Meeting minutes
                Source: \(inputToken), 2026-07-22, Asia/Shanghai

                ## Explicit decisions
                - Launch the beta on 2026-08-05. Evidence: 09:12, Chair.

                ## Accepted action items
                | Owner | Action | Deadline | Evidence |
                | --- | --- | --- | --- |
                | Mira | Deliver the QA report | 2026-07-28 17:00 Asia/Shanghai | 09:18 explicit commitment |

                ## Open items
                - Rollback drill owner: unassigned. Evidence: 09:30, Chair.

                ## Proposal, not a decision
                - Chen proposed moving the documentation launch earlier at 09:23; no decision was made.
                """,
                ["MEETING-7301", "Launch the beta on 2026-08-05", "Mira", "2026-07-28 17:00 Asia/Shanghai", "unassigned", "Proposal, not a decision", "no decision was made"]
            )
        case .scheduleTasks:
            return json(
                "outputs/schedule-plan.json",
                [
                    "mode": "plan_only",
                    "timezone": "Asia/Shanghai",
                    "writes_performed": false,
                ],
                ["plan_only", "Asia/Shanghai", "writes_performed"]
            )
        case .knowledgeMemory:
            return json(
                "outputs/memory-verification.json",
                [
                    "status": "stored_and_recalled",
                    "scope": "global",
                    "kind": "preference",
                    "content": inputToken,
                    "provenance": "explicit user statement in the current conversation",
                    "sensitivity": "normal",
                    "recall_verification": "passed",
                ],
                ["stored_and_recalled", "global", "preference", inputToken, "explicit user statement", "normal", "passed"]
            )
        case .fileOrganization:
            return json(
                "outputs/file-organization-dry-run.json",
                [
                    "mode": "dry-run",
                    "source": inputToken,
                    "operations": [["action": "move", "collision": "none"]],
                    "symlink_policy": "do not follow",
                    "hash_equality": "required before deduplication",
                    "rollback": "manifest-backed",
                ],
                ["dry-run", "collision", "symlink_policy", "hash_equality", "rollback"]
            )
        case .openSourceModel:
            return json(
                "outputs/model-installation-plan.json",
                [
                    "requested_model": inputToken,
                    "minimum_context": 65_536,
                    "selected_variant": "publisher/model-4bit-mlx",
                    "runtime": "mlx",
                    "estimated_peak_memory_bytes": 6_442_450_944,
                    "available_memory_bytes": 17_179_869_184,
                    "revision": "immutable-fixture-revision",
                    "checksum_policy": "SHA-256 required before configuration",
                    "status": "planned; download not started",
                ],
                [inputToken, "65536", "selected_variant", "mlx", "estimated_peak_memory_bytes", "SHA-256", "download not started"]
            )
        case .automations:
            return json(
                "outputs/automation-draft.json",
                [
                    "enabled": false,
                    "source": inputToken,
                    "idempotency_key": "fixture-73",
                    "retry_cap": 3,
                    "checkpoint": "before external write",
                    "dry_run": true,
                    "structured_history": true,
                ],
                ["idempotency_key", "retry_cap", "checkpoint", "dry_run", "structured_history"]
            )
        case .email:
            return SkillEndToEndArtifact(
                relativePath: "outputs/reply-draft.eml",
                content: """
                From: assistant@example.invalid
                To: recipient@example.invalid
                Subject: Re: Fixture review
                X-Nola-Draft: true
                X-Correlation-Key: fixture-73

                Draft only. Nothing was sent.
                Evidence reviewed: \(inputToken)
                """,
                format: .email,
                requiredFragments: ["To:", "Subject: Re:", "X-Nola-Draft: true", "Draft only. Nothing was sent."]
            )
        case .academicReader:
            return markdown(
                "outputs/paper-evidence-map.md",
                "# Evidence map\n\nResearch question: \(inputToken)\n\n## Method\nObservational fixture review.\n\n## Evidence\nThe source reports 73 observations.\n\n## Limitations\nNo causal inference is supported.\n",
                ["# Evidence map", "Research question", "## Method", "## Evidence", "## Limitations", "No causal inference"]
            )
        case .scientificWriting:
            return markdown(
                "outputs/scientific-section.md",
                "# Results\n\nThe supplied evidence (\(inputToken)) reports 73 observations.\n\n# Interpretation\nThe estimate is descriptive and remains subject to sampling uncertainty.\n",
                ["# Results", inputToken, "73 observations", "# Interpretation", "sampling uncertainty"]
            )
        case .academicPolishing:
            return markdown(
                "outputs/polished-passage.md",
                "# Polished passage\n\nThe analysis includes exactly 73 orchard pears (\(inputToken)).\n\n## Change summary\nImproved clarity and grammar; scientific meaning and quantities were preserved.\n",
                ["# Polished passage", "73 orchard pears", "## Change summary", "meaning and quantities were preserved"]
            )
        case .manuscriptReviewer:
            return markdown(
                "outputs/manuscript-review.md",
                "# Manuscript review\n\nSource: \(inputToken)\n\n## Major concern\nThe sampling procedure is not reproducible from the supplied description.\n\n## Actionable revision\nReport inclusion criteria, exclusions, and the complete analysis protocol.\n",
                ["# Manuscript review", "## Major concern", "not reproducible", "## Actionable revision", "inclusion criteria"]
            )
        case .reviewerResponse:
            return markdown(
                "outputs/reviewer-response.md",
                "# Response to reviewers\n\n## Comment 1\nPlease clarify the sample count.\n\n## Response\nThank you. We now report 73 observations and explain exclusions.\n\n## Manuscript change\nMethods, paragraph 2. Source: \(inputToken)\n",
                ["# Response to reviewers", "## Comment 1", "## Response", "73 observations", "## Manuscript change", "Methods, paragraph 2"]
            )
        case .statisticsAudit:
            return markdown(
                "outputs/statistics-audit.md",
                "# Statistics audit\n\nSource: \(inputToken)\n\n## Estimand\nDifference in observed group means.\n\n## Assumptions and uncertainty\nIndependence is unverified; report confidence intervals and effect size.\n\n## Multiplicity\nNo correction strategy was documented.\n",
                ["# Statistics audit", "## Estimand", "## Assumptions and uncertainty", "confidence intervals", "effect size", "## Multiplicity"]
            )
        case .referenceVerifier:
            return markdown(
                "outputs/reference-verification.md",
                "# Reference verification\n\nSource: \(inputToken)\n\n| Reference | Identifier | Metadata | Claim support | Status |\n| --- | --- | --- | --- | --- |\n| Fixture study | DOI unresolved | title matched | partial | unresolved |\n\nRetraction status: not verified; do not treat this entry as confirmed.\n",
                ["# Reference verification", "Identifier", "Metadata", "Claim support", "unresolved", "Retraction status"]
            )
        case .paperToPresentation:
            return markdown(
                "outputs/research-presentation.md",
                "# Slide 1: Research question\n\n\(inputToken)\n\nSpeaker notes: Introduce the question and study scope.\n\n---\n\n# Slide 2: Evidence and limitations\n\n- 73 observations\n- No causal inference\n\nSpeaker notes: Link each claim to the source and state uncertainty.\n",
                ["# Slide 1:", "# Slide 2:", "Speaker notes:", "73 observations", "No causal inference", "---"]
            )
        }
    }

    func validate(at url: URL) throws {
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw validationError("artifact does not exist: \(url.path)")
        }
        let text = try String(contentsOf: url, encoding: .utf8)
        for fragment in requiredFragments where !text.localizedCaseInsensitiveContains(fragment) {
            throw validationError("artifact is missing required evidence: \(fragment)")
        }
        switch format {
        case .json:
            guard let data = text.data(using: .utf8),
                  (try? JSONSerialization.jsonObject(with: data)) != nil else {
                throw validationError("artifact is not valid JSON")
            }
        case .csv:
            let rows = text.split(separator: "\n")
            guard rows.count >= 2, rows.allSatisfy({ $0.contains(",") }) else {
                throw validationError("artifact is not a populated CSV")
            }
        case .email:
            guard text.contains("\n\n") else {
                throw validationError("email draft has no header/body boundary")
            }
        case .markdown:
            guard text.contains("#") else {
                throw validationError("artifact has no Markdown structure")
            }
        }
    }

    private static func markdown(
        _ path: String,
        _ content: String,
        _ requiredFragments: [String]
    ) -> SkillEndToEndArtifact {
        SkillEndToEndArtifact(
            relativePath: path,
            content: content,
            format: .markdown,
            requiredFragments: requiredFragments
        )
    }

    private static func csv(
        _ path: String,
        _ content: String,
        _ requiredFragments: [String]
    ) -> SkillEndToEndArtifact {
        SkillEndToEndArtifact(
            relativePath: path,
            content: content,
            format: .csv,
            requiredFragments: requiredFragments
        )
    }

    private static func json(
        _ path: String,
        _ object: [String: Any],
        _ requiredFragments: [String]
    ) -> SkillEndToEndArtifact {
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys])
        return SkillEndToEndArtifact(
            relativePath: path,
            content: String(decoding: data, as: UTF8.self),
            format: .json,
            requiredFragments: requiredFragments
        )
    }

    private func validationError(_ message: String) -> NSError {
        NSError(
            domain: "NolaMacTests.SkillArtifactValidation",
            code: 1,
            userInfo: [NSLocalizedDescriptionKey: message]
        )
    }

}

/// Owns only this fixture's Run. Cancellation uses the production path; no
/// fabricated terminal event or automatic answer can turn a hung test green.
@MainActor
private final class SkillEndToEndEventConsumer {
    private let request: MacLocalAgentRunRequest
    private let continuation: AsyncThrowingStream<MacLocalAgentEvent, Error>.Continuation
    private var reader: Task<Void, Never>?
    private var deadline: Task<Void, Never>?
    private var failure: Error?
    private var finished = false

    init(request: MacLocalAgentRunRequest, continuation: AsyncThrowingStream<MacLocalAgentEvent, Error>.Continuation) {
        self.request = request
        self.continuation = continuation
    }

    func start(timeoutSeconds: Int) {
        deadline = Task { @MainActor in
            do { try await Task.sleep(for: .seconds(timeoutSeconds)) } catch { return }
            await abort("Skill fixture exceeded its whole-Run deadline")
        }
        reader = Task { @MainActor in
            do {
                for try await event in await MacAgentHostRuntime.shared.run(request) {
                    if event.kind == .userInputRequested, failure == nil {
                        deadline?.cancel()
                        deadline = Task { @MainActor in
                            await abort("Unexpected user decision in noninteractive Skill fixture: \(event.title ?? "request_user_input")")
                        }
                    } else if failure == nil {
                        continuation.yield(event)
                    }
                }
                finish(failure)
            } catch {
                finish(failure ?? error)
            }
        }
    }

    private func abort(_ message: String) async {
        guard !finished else { return }
        failure = NSError(domain: "NolaMacTests.SkillRunDeadline", code: 1,
                          userInfo: [NSLocalizedDescriptionKey: message])
        // Do not put the drain deadline behind acquisition of the runtime
        // actor. Its cancel implementation interrupts immediately and owns
        // five-second escalation; a busy actor must not block this test timer.
        Task { await MacAgentHostRuntime.shared.cancel(conversationID: request.conversationID) }
        // Runtime owns a five-second process escalation. Allow its canonical
        // cancellation/seal to drain, then fail even if the stream stays open.
        do { try await Task.sleep(for: .seconds(6)) } catch { return }
        finish(failure)
    }

    private func finish(_ error: Error?) {
        guard !finished else { return }
        finished = true
        deadline?.cancel()
        reader?.cancel()
        if let error { continuation.finish(throwing: error) }
        else { continuation.finish() }
        reader = nil
        deadline = nil
    }

    func stop() {
        if !finished {
            Task { await MacAgentHostRuntime.shared.cancel(conversationID: request.conversationID) }
        }
        finish(CancellationError())
    }
}

private actor SkillEndToEndExecutionGate {
    private var isAcquired = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    func acquire() async {
        if !isAcquired {
            isAcquired = true
            return
        }
        await withCheckedContinuation { continuation in
            waiters.append(continuation)
        }
    }

    func release() {
        if waiters.isEmpty {
            isAcquired = false
        } else {
            waiters.removeFirst().resume()
        }
    }
}

private struct SkillEndToEndMockReport {
    var stages: [Int] = []
    var failures: [String] = []
    var evidence: [String] = []
    var rootWasVisible = false
    var skillContentWasLoaded = false
    var inputWasRead = false
    var outputWasWritten = false
}

private final class SkillEndToEndMockServer: @unchecked Sendable {
    private let process: Process
    private let standardOutput: Pipe
    private let standardError: Pipe
    private let reportURL: URL
    private let configurationURL: URL
    private let runControlURL: URL
    private(set) var endpoint = ""

    init(configuration: [String: Any], workspace: URL) throws {
        let fixtureRoot = workspace.appendingPathComponent(".skill-e2e", isDirectory: true)
        try FileManager.default.createDirectory(at: fixtureRoot, withIntermediateDirectories: true)
        configurationURL = fixtureRoot.appendingPathComponent("configuration.json")
        runControlURL = fixtureRoot.appendingPathComponent("run-control.json")
        reportURL = fixtureRoot.appendingPathComponent("report.jsonl")
        var serverConfiguration = configuration
        serverConfiguration["report_path"] = reportURL.path
        serverConfiguration["run_control_path"] = runControlURL.path
        try JSONSerialization.data(withJSONObject: ["generation": UUID().uuidString, "phase": "creation"])
            .write(to: runControlURL, options: .atomic)
        let configurationData = try JSONSerialization.data(
            withJSONObject: serverConfiguration,
            options: [.sortedKeys]
        )
        try configurationData.write(to: configurationURL, options: .atomic)

        process = Process()
        standardOutput = Pipe()
        standardError = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = ["-u", "-c", Self.pythonServer, configurationURL.path]
        process.standardOutput = standardOutput
        process.standardError = standardError
        process.standardInput = FileHandle.nullDevice
        try process.run()

        let portLine = try Self.readLine(
            from: standardOutput.fileHandleForReading,
            process: process,
            errorHandle: standardError.fileHandleForReading
        )
        guard portLine.hasPrefix("PORT "),
              let port = UInt16(portLine.dropFirst(5)) else {
            stop()
            throw NSError(
                domain: "NolaMacTests.SkillEndToEndMockServer",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Mock LLM did not publish a port: \(portLine)"]
            )
        }
        endpoint = "http://127.0.0.1:\(port)/v1"
    }

    deinit {
        stop()
    }

    func beginExecutionRun() throws {
        // Test control, not model history or an execution authorization. The
        // next Run must establish its owner using a newly issued exact call.
        try JSONSerialization.data(withJSONObject: ["generation": UUID().uuidString, "phase": "execution"])
            .write(to: runControlURL, options: .atomic)
    }

    func stop() {
        guard process.isRunning else { return }
        process.terminate()
        let deadline = Date().addingTimeInterval(1)
        while process.isRunning && Date() < deadline {
            Thread.sleep(forTimeInterval: 0.01)
        }
        if process.isRunning {
            // This Process was created by this fixture, never a name/PID scan.
            Darwin.kill(process.processIdentifier, SIGKILL)
        }
    }

    func report() throws -> SkillEndToEndMockReport {
        guard FileManager.default.fileExists(atPath: reportURL.path) else {
            return SkillEndToEndMockReport(failures: ["mock provider produced no report"])
        }
        let content = try String(contentsOf: reportURL, encoding: .utf8)
        var report = SkillEndToEndMockReport()
        for line in content.split(separator: "\n") {
            guard let data = line.data(using: .utf8),
                  let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let stage = object["stage"] as? Int else {
                continue
            }
            if report.stages.last != stage {
                report.stages.append(stage)
            }
            let checks = object["checks"] as? [String: Bool] ?? [:]
            for (name, passed) in checks where !passed {
                report.failures.append("stage \(stage): \(name)")
            }
            if let evidence = object["evidence"] as? String, !evidence.isEmpty {
                report.evidence.append("stage \(stage): \(evidence)")
            }
            report.rootWasVisible = report.rootWasVisible
                || (object["root_visible"] as? Bool ?? false)
            report.skillContentWasLoaded = report.skillContentWasLoaded
                || checks["skill_loaded"] == true
            report.inputWasRead = report.inputWasRead
                || checks["input_read"] == true
                || checks["artifact_source_written"] == true
            report.outputWasWritten = report.outputWasWritten
                || checks["output_written"] == true
        }
        return report
    }

    private static func readLine(
        from handle: FileHandle,
        process: Process,
        errorHandle: FileHandle
    ) throws -> String {
        var data = Data()
        let deadline = Date().addingTimeInterval(8)
        while Date() < deadline {
            let chunk = handle.availableData
            if chunk.isEmpty {
                let error = String(decoding: errorHandle.availableData, as: UTF8.self)
                throw NSError(
                    domain: "NolaMacTests.SkillEndToEndMockServer",
                    code: 2,
                    userInfo: [NSLocalizedDescriptionKey: "Mock LLM exited before startup: \(error)"]
                )
            }
            data.append(chunk)
            if let newline = data.firstIndex(of: 0x0A) {
                return String(decoding: data[..<newline], as: UTF8.self)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
            }
            if !process.isRunning { break }
        }
        throw NSError(
            domain: "NolaMacTests.SkillEndToEndMockServer",
            code: 3,
            userInfo: [NSLocalizedDescriptionKey: "Timed out waiting for the mock LLM port"]
        )
    }

    private static let pythonServer = #"""
import json
import os
import pathlib
import urllib.parse
import re
import shlex
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    CONFIG = json.load(handle)

LOCK = threading.Lock()
REQUEST_LOCK = threading.RLock()
RUN_STATES = {}
CURRENT_RUN_STATE = None
CURRENT_REQUEST_KEY = None
STAGE = 0
LAST_PROVISIONAL_FINAL = None
POST_GENERATION_CHECK = None
CURRENT_HOST_TOOL_NAMES = []
CURRENT_RESPONSE_STAGE = 0

def select_run_state(body):
    # Swift explicitly starts each fixture Run. Historical user/tool messages
    # never choose a Run or advance the active ledger.
    global CURRENT_RUN_STATE, CURRENT_REQUEST_KEY, CURRENT_RESPONSE_STAGE, STAGE, LAST_PROVISIONAL_FINAL, POST_GENERATION_CHECK
    with open(CONFIG["run_control_path"], encoding="utf-8") as handle:
        control = json.load(handle)
    key = control["generation"]
    state = RUN_STATES.get(key)
    if state is None:
        execution = control["phase"] == "execution"
        # Creation can require extra validation/Skill completion rounds. Keep
        # report numbering continuous without assuming it always takes 3 turns.
        # The new Run still gets its own owner, response ledger and pending call.
        first_stage = max((existing["stage"] for existing in RUN_STATES.values()), default=3) if execution else 0
        state = {"owner": None, "generation": key, "phase": control["phase"], "stage": first_stage,
                 "candidate": None, "validation": None, "pending": None, "responses": {}}
        RUN_STATES[key] = state
    CURRENT_RUN_STATE = state
    CURRENT_REQUEST_KEY = json.dumps(body, sort_keys=True, ensure_ascii=False)
    STAGE, LAST_PROVISIONAL_FINAL, POST_GENERATION_CHECK = state["stage"], state["candidate"], state["validation"]
    CURRENT_RESPONSE_STAGE = STAGE
    return state

def append_report(stage, checks, evidence="", observations=None):
    record = {"stage": stage, "checks": checks}
    if evidence:
        record["evidence"] = evidence[:1200]
    if observations:
        record.update(observations)
    with open(CONFIG["report_path"], "a", encoding="utf-8") as handle:
        handle.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n")
        handle.flush()

def tool_response(name, arguments, call_id):
    global STAGE
    if (
        name != "tool_search"
        and name not in CURRENT_HOST_TOOL_NAMES
        and "tool_search" in CURRENT_HOST_TOOL_NAMES
    ):
        with LOCK:
            STAGE = CURRENT_RESPONSE_STAGE
        arguments = {"query": "select:" + name}
        call_id = "call_discover_" + str(CURRENT_RESPONSE_STAGE) + "_" + name
        name = "tool_search"
    call_id += "_" + CURRENT_RUN_STATE["generation"]
    return {
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": json.dumps(arguments, ensure_ascii=False, separators=(",", ":"))
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    }

def final_response(content):
    return {
        "choices": [{
            "index": 0,
            "delta": {"content": content},
            "finish_reason": "stop"
        }]
    }

def pending_skill_updates(messages_text, completed_tools):
    output_path = (
        CONFIG.get("artifact_output_absolute_path")
        or CONFIG.get("execution_output_absolute_path")
    )
    if not output_path:
        return []
    def artifact_ref_for_run(run_id):
        selected_path = (
            CONFIG.get("automation_persistence_path")
            if CONFIG.get("automation_persistence_path") and CURRENT_RUN_STATE["phase"] == "creation"
            else output_path
        )
        return pathlib.Path(selected_path).resolve().as_uri()
    updates = []
    for run_id, step_id in re.findall(r"skill_update run_id=([^\s]+) action=progress step_id=([^\s]+) completed=true", messages_text):
        updates.append({"run_id": run_id, "action": "progress", "step_id": step_id, "completed": True})
    for run_id, artifact_id in re.findall(r"skill_update run_id=([^\s]+) action=artifact artifact_id=([^\s]+) artifact_ref=", messages_text):
        updates.append({
            "run_id": run_id,
            "action": "artifact",
            "artifact_id": artifact_id,
            "artifact_ref": artifact_ref_for_run(run_id)
        })
    source_call_id = next((str(message.get("tool_call_id") or message.get("call_id"))
        for message in reversed(completed_tools)
        if (message.get("tool_call_id") or message.get("call_id"))
        and not str(message.get("tool_call_id") or message.get("call_id")).startswith("call_skill_update_")), None)
    for run_id, verifier_id in re.findall(r"skill_update run_id=([^\s]+) action=verification verifier_id=([^\s]+) passed=true evidence=", messages_text):
        if source_call_id is None:
            continue
        updates.append({
            "run_id": run_id,
            "action": "verification",
            "verifier_id": verifier_id,
            "passed": True,
            "evidence": {"kind": "model_judgment", "sources": [{"kind": "tool_call", "call_id": source_call_id}]},
            "summary": "Model review of the preceding retained output check."
        })
    unique = []
    seen = set()
    for update in updates:
        key = json.dumps(update, ensure_ascii=False, sort_keys=True)
        if key not in seen:
            seen.add(key)
            unique.append(update)
    return unique

def skill_updates_response(updates):
    # `skill_update` mutates one ordered Core-owned SkillRun. Emit exactly one
    # record per model turn so providers cannot present dependent state changes
    # as a parallel tool-call batch.
    update = updates[0]
    return {
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call_skill_update_" + update["action"] + "_" + str(STAGE) + "_" + CURRENT_RUN_STATE["generation"],
                    "type": "function",
                    "function": {
                        "name": "skill_update",
                        "arguments": json.dumps(update, ensure_ascii=False, separators=(",", ":"))
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    }

def skill_is_advertised(messages, skill_id):
    catalog_markers = (
        "nola_skill_catalog_v1",
        "built-in skills for nola mac",
        "skills are available for use with the skill tool",
    )
    for message in messages:
        serialized = json.dumps(message, ensure_ascii=False, sort_keys=True)
        lowered = serialized.lower()
        if skill_id in serialized and any(marker in lowered for marker in catalog_markers):
            return True
    return False

def skill_context_loaded(messages_text, last_tool, skill_id):
    activation_receipt = str(last_tool.get("content", ""))
    return (
        (
            skill_id in activation_receipt
            and "skill_activated" in activation_receipt
        )
        or (
            skill_id in messages_text
            and "skill_activated" in messages_text
        )
    )

def validation_target_paths(path):
    paths = [path]
    if CONFIG.get("artifact_mode") == "file_organization" and path == CONFIG.get("artifact_output_absolute_path"):
        with open(path, encoding="utf-8") as handle:
            manifest = json.load(handle)
        for row in manifest.get("operations", []):
            if row.get("status") in ("planned", "executed", "rolled_back"):
                for key in ("source", "final_destination"):
                    target = row.get(key)
                    if target and target not in paths:
                        paths.append(target)
    for key in ("artifact_spec_absolute_path", "artifact_source_absolute_path"):
        candidate = CONFIG.get(key)
        if candidate and os.path.isfile(candidate) and candidate not in paths:
            paths.append(candidate)
    return paths

def output_validation_response(path, call_id):
    # Reading bytes is not verification evidence. Run a separate foreground
    # check over the exact post-mutation resource so Host attests its version.
    targets = validation_target_paths(path)
    command = " && ".join("/bin/test -s " + shlex.quote(target) for target in targets)
    if CONFIG.get("artifact_mode") == "file_organization":
        command = "/usr/bin/python3 " + shlex.quote(CONFIG["artifact_generator_absolute_path"]) + " --validate-only --expected-state rolled_back --manifest " + shlex.quote(path)
    return tool_response(
        "bash",
        {
            "command": command,
            "cwd": CONFIG["workspace_path"],
            "operation_kind": "validate",
            "validation_targets": targets
        },
        call_id
    )

def output_validation_succeeded(tool_evidence, path):
    try:
        message = json.loads(tool_evidence)
        result = message.get("content", message.get("output", {}))
        if isinstance(result, str):
            result = json.loads(result)
        receipt = result.get("_nola_execution", {})
        expected = {resource_entry_uri(target) for target in validation_target_paths(path)}
        verified = {canonical_file_resource_uri(check.get("resource_uri")) for check in receipt.get("checks", []) if
            check.get("passed") is True
            and isinstance(check.get("version"), str) and check["version"]
            and check.get("predicate") == "process_exit_zero_with_stable_resource"
        }
        return receipt.get("state") == "completed" and expected.issubset(verified)
    except (ValueError, TypeError, AttributeError):
        return False

def canonical_file_resource_uri(uri):
    # WHATWG file URLs (Rust) leave parentheses literal; pathlib.as_uri
    # percent-encodes them. Compare the same file identity, not URL spelling.
    # No filesystem resolution here: a symlink remains its own directory entry.
    parsed = urllib.parse.urlsplit(uri)
    if parsed.scheme != "file" or parsed.netloc or parsed.query or parsed.fragment:
        raise ValueError("Expected a local file resource URI")
    path = pathlib.Path(os.fsdecode(urllib.parse.unquote_to_bytes(parsed.path)))
    if not path.is_absolute():
        raise ValueError("Expected an absolute file resource path")
    return path.as_uri()

def resource_entry_uri(raw_path):
    # A moved/deleted symlink is its own directory entry, not the file it points
    # at. Canonicalize parents while preserving the observed leaf identity.
    path = pathlib.Path(raw_path if os.path.isabs(raw_path)
                        else os.path.join(CONFIG["workspace_path"], raw_path))
    return (path.parent.resolve() / path.name).as_uri()

def artifact_command_args(command, path, operation="create"):
    validation_targets = [path]
    for source_key in ("artifact_spec_absolute_path", "artifact_source_absolute_path"):
        source = CONFIG.get(source_key)
        if source and os.path.exists(source) and source not in validation_targets:
            validation_targets.append(source)
    return {
        "command": command,
        "cwd": CONFIG["workspace_path"],
        "operation_kind": "mutate_and_validate",
        "validation_targets": validation_targets,
        "output_artifacts": [{"path": path, "operation": operation}]
    }

def organization_mutation_args(command, rollback=False):
    path = CONFIG["artifact_output_absolute_path"]
    with open(path, encoding="utf-8") as handle:
        manifest = json.load(handle)
    effects = [{"path": path, "operation": "edit"}]
    for row in manifest.get("operations", []):
        if row.get("status") != ("executed" if rollback else "planned"):
            continue
        effects.extend([
            {"path": row["source"], "operation": "create" if rollback else "delete"},
            {"path": row["final_destination"], "operation": "delete" if rollback else "create"}
        ])
    return {"command": command, "cwd": CONFIG["workspace_path"],
            "operation_kind": "mutate", "output_artifacts": effects}

def is_synthesis_tool_surface(tool_names):
    internal_evidence_tools = {
        "tool_output_find",
        "tool_output_list",
        "tool_output_read",
        "tool_output_search",
    }
    return all(name in internal_evidence_tools for name in tool_names)

class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, format, *args):
        return

    def do_GET(self):
        if self.path.startswith("/source/primary"):
            self.send_html("""
            <html><head><title>Orbital Standards Agency release</title></head><body>
            <main><h1>Orbital battery standard enters compliance in July 2026</h1>
            <p>Published 2026-07-20 by the Orbital Standards Agency.</p>
            <p>The primary standard sets the mandatory compliance date at July 2026 after public review.</p>
            </main></body></html>
            """)
            return
        if self.path.startswith("/source/secondary"):
            self.send_html("""
            <html><head><title>Independent Laboratory bulletin</title></head><body>
            <main><h1>Independent laboratory completes orbital battery checks</h1>
            <p>Published 2026-07-21 by the Independent Energy Laboratory.</p>
            <p>The laboratory confirmed 73 completed cycles. Testing covered 20–25 °C, so results do not establish performance outside that range.</p>
            </main></body></html>
            """)
            return
        payload = {
            "object": "list",
            "data": [{
                "id": "nola-skill-e2e",
                "object": "model",
                "context_window": 128000,
                "max_model_len": 128000
            }]
        }
        self.send_json(payload)

    def do_POST(self):
        # Globals below are only a view of one ledger. Serialize fixture
        # requests so a concurrent retry cannot borrow another Run's state.
        with REQUEST_LOCK:
            self.handle_model_post()

    def handle_model_post(self):
        global STAGE, LAST_PROVISIONAL_FINAL, POST_GENERATION_CHECK, CURRENT_HOST_TOOL_NAMES, CURRENT_RESPONSE_STAGE
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        try:
            body = json.loads(raw.decode("utf-8"))
        except Exception:
            self.send_json({"error": {"message": "invalid test request"}}, status=400)
            return

        if self.path.endswith("/search"):
            query = body.get("q", "")
            origin = "http://127.0.0.1:" + str(self.server.server_address[1])
            organic = []
            for rank in range(1, 21):
                if rank == 1:
                    title = "IANA-managed Reserved Domains"
                    link = "https://www.iana.org/help/example-domains"
                    snippet = "Official explanation of example domains reserved for documentation."
                elif rank == 2:
                    title = "Example Domain"
                    link = "https://example.com/"
                    snippet = "Reserved domain page intended for illustrative examples."
                else:
                    title = "Nola Research Result " + str(rank)
                    link = "https://example.com/search-result-" + str(rank)
                    snippet = "Ranked discovery result " + str(rank) + " for " + query
                organic.append({"position": rank, "title": title, "link": link, "snippet": snippet, "date": "2026-07-21"})
            self.send_json({"searchParameters": {"q": query}, "organic": organic})
            return

        state = select_run_state(body)
        cached = state["responses"].get(CURRENT_REQUEST_KEY)
        if cached is not None:
            self.write_payload(cached, body)
            return
        pending = state["pending"]
        if pending is not None:
            items = body.get("messages") or body.get("input") or []
            receipts = [item for item in items if isinstance(item, dict)
                        and (item.get("role") == "tool" or item.get("type") == "function_call_output")]
            receipt = receipts[-1] if receipts else {}
            call_id = str(receipt.get("tool_call_id") or receipt.get("call_id") or "")
            call_matches = call_id.endswith("::" + pending["id"])
            # Only a receipt for this generation's newly issued exact call can
            # establish the owner. An old call in history cannot do so.
            if state["owner"] is None and call_matches and ":host_effect:" in call_id:
                state["owner"] = call_id.split(":host_effect:", 1)[0]
            owner_matches = state["owner"] is not None and call_id.startswith(state["owner"] + ":host_effect:")
            name_matches = not receipt.get("name") or receipt.get("name") == pending["name"]
            if not (owner_matches and call_matches and name_matches and ("content" in receipt or "output" in receipt)):
                # A transport retry or unrelated receipt cannot complete this
                # call. Reissue the same ID; the real Host owns deduplication.
                self.write_payload(pending["payload"], body)
                return
            expected_artifacts = pending.get("artifacts", [])
            if expected_artifacts:
                result = receipt.get("content", receipt.get("output"))
                try:
                    result = json.loads(result) if isinstance(result, str) else result
                    execution = result.get("_nola_execution", {})
                    actual = {canonical_file_resource_uri(effect.get("resource_uri")): effect for effect in execution.get("effects", [])}
                    expected = [resource_entry_uri(item["path"]) for item in expected_artifacts]
                    observed = execution.get("state") == "completed" and all(
                        uri in actual and isinstance(actual[uri].get("after_version"), str)
                        for uri in expected)
                except (ValueError, TypeError, AttributeError):
                    observed = False
                append_report(STAGE, {"declared_mutations_have_host_versions": observed})
            state["pending"] = None

        body_text = json.dumps(body, ensure_ascii=False, sort_keys=True)
        # Keep private completion phases separate from scripted business steps.
        system_text = "\n\n".join(
            item.get("content", "") for item in body.get("messages", [])
            if item.get("role") == "system" and isinstance(item.get("content"), str)
        )
        candidate_start = "<untrusted_answer_candidate>\n"
        candidate_end = "\n</untrusted_answer_candidate>"
        if candidate_start in system_text and candidate_end in system_text:
            candidate = system_text.split(candidate_start, 1)[1].split(candidate_end, 1)[0]
            if "This is an independent constraint-audit boundary." in system_text:
                self.send_payload(final_response(json.dumps({
                    "decision": "accept", "constraints": ["scripted skill scenario"],
                    "items": [{"label": candidate, "eligibility": "eligible",
                               "evidence": "Business steps assert fixture outputs and verification"}],
                    "violations": []
                })), body)
                return
            if "This is the final presentation boundary." in system_text:
                with LOCK:
                    LAST_PROVISIONAL_FINAL = None
                self.send_payload(final_response(candidate), body)
                return
        scope_qualification = "NOLA_SCOPE_QUALIFICATION_V1" in body_text
        messages_text = body_text
        tools_text = body_text
        host_tool_names = [
            (item.get("function", {}).get("name", "") or item.get("name", ""))
            for item in body.get("tools", [])
            if isinstance(item, dict)
        ]
        request_items = body.get("messages") or body.get("input") or []
        if not isinstance(request_items, list):
            request_items = []
        user_items = [
            item for item in request_items
            if isinstance(item, dict) and item.get("role") == "user"
        ]
        latest_user_text = (
            json.dumps(user_items[-1], ensure_ascii=False, sort_keys=True)
            if user_items else ""
        )
        tool_messages = [
            message for message in request_items
            if isinstance(message, dict)
            and (
                message.get("role") == "tool"
                or message.get("type") == "function_call_output"
            )
        ]
        last_tool = tool_messages[-1] if tool_messages else {}
        tool_evidence = json.dumps(last_tool, ensure_ascii=False, sort_keys=True)
        last_tool_name = str(last_tool.get("name", ""))
        if not last_tool_name and last_tool.get("type") == "function_call_output":
            completed_call_id = str(last_tool.get("call_id", ""))
            for item in reversed(request_items):
                if (
                    isinstance(item, dict)
                    and item.get("type") == "function_call"
                    and str(item.get("call_id", "")) == completed_call_id
                ):
                    last_tool_name = str(item.get("name", ""))
                    break
        # Read only the current Core system contract, never quoted tool/user
        # history. Successful updates disappear from the next pending list;
        # merely having sent a call is not proof that Core accepted it.
        skill_updates = pending_skill_updates(system_text, tool_messages)
        if LAST_PROVISIONAL_FINAL is None:
            skill_updates = []
        post_generation_verified = False
        if POST_GENERATION_CHECK is not None:
            if last_tool_name == "tool_search":
                # Discovery is not a validation receipt. Submit the now-visible
                # validator without advancing the business script.
                CURRENT_HOST_TOOL_NAMES = host_tool_names
                self.send_payload(output_validation_response(
                    POST_GENERATION_CHECK, "call_post_generation_check_discovered_" + str(STAGE)), body)
                return
            post_generation_verified = output_validation_succeeded(tool_evidence, POST_GENERATION_CHECK)
            append_report(STAGE, {"post_generation_version_checked": post_generation_verified}, tool_evidence)
            POST_GENERATION_CHECK = None
            if not post_generation_verified:
                # Preserve a visible fixture failure instead of certifying
                # progress/verifiers from an unobserved check.
                self.send_payload(final_response("Fixture post-generation verification failed"), body)
                return
        regression_mode = CONFIG.get("regression_mode", "")
        expect_failure = bool(CONFIG.get("expect_skill_load_failure", False))

        synthesis_content = None
        if scope_qualification:
            stage = -1
        else:
            with LOCK:
                if (last_tool_name == "skill_update" or post_generation_verified) and LAST_PROVISIONAL_FINAL is not None and not skill_updates:
                    synthesis_content = LAST_PROVISIONAL_FINAL
                    stage = STAGE
                    STAGE += 1
                elif LAST_PROVISIONAL_FINAL is not None and not skill_updates:
                    synthesis_content = LAST_PROVISIONAL_FINAL
                    stage = STAGE
                else:
                    stage = STAGE
                    STAGE += 1

        CURRENT_HOST_TOOL_NAMES = host_tool_names
        CURRENT_RESPONSE_STAGE = stage

        if synthesis_content is not None:
            append_report(
                stage,
                {"skill_completion_protocol_completed": not skill_updates
                    and (last_tool_name == "skill_update" or post_generation_verified)},
                tool_evidence
            )
            payload = final_response(synthesis_content)
        elif skill_updates:
            append_report(
                stage,
                {"skill_completion_protocol_requested": "skill_update" in host_tool_names},
                json.dumps(skill_updates, ensure_ascii=False, sort_keys=True)
            )
            payload = skill_updates_response(skill_updates)
        elif scope_qualification:
            payload = final_response('{"scope":"specific"}')
        elif regression_mode:
            if stage == 0:
                append_report(
                    stage,
                    {
                        "user_prompt_preserved": CONFIG["prompt"] in messages_text,
                        "protocol_regression_has_skill_router": "Skill" in host_tool_names,
                        "protocol_regression_host_tools_are_unique": len(host_tool_names) == len(set(host_tool_names))
                    }
                )
                if regression_mode == "multi_turn_boundary":
                    payload = tool_response(
                        "file_tree",
                        {"root": "."},
                        "call_boundary_list_root"
                    )
                    payload["choices"][0]["delta"]["content"] = "INTERNAL_TOOL_PLAN_ONE"
                    content = None
                elif regression_mode == "object_write":
                    payload = tool_response(
                        "write_file",
                        {
                            "path": CONFIG["output_path"],
                            "content": "Object-shaped arguments reached the real write tool."
                        },
                        "call_object_write"
                    )
                    function = payload["choices"][0]["delta"]["tool_calls"][0]["function"]
                    if function["name"] == "write_file":
                        function["arguments"] = {
                            "path": CONFIG["output_path"],
                            "content": "Object-shaped arguments reached the real write tool."
                        }
                    content = None
                elif regression_mode == "double_encoded_write":
                    payload = tool_response(
                        "write_file",
                        {
                            "path": CONFIG["output_path"],
                            "content": "Double-encoded arguments reached the real write tool."
                        },
                        "call_double_encoded_write"
                    )
                    function = payload["choices"][0]["delta"]["tool_calls"][0]["function"]
                    if function["name"] == "write_file":
                        function["arguments"] = json.dumps(function["arguments"])
                    content = None
                else:
                    payload = tool_response(
                        "write_file",
                        {},
                        "call_invalid_empty_write"
                    )
            elif stage == 1 and regression_mode == "multi_turn_boundary":
                append_report(
                    stage,
                    {
                        "first_tool_result_reached_model": (
                            "file_tree" in messages_text
                            and '"error":true' not in tool_evidence.lower()
                            and '"error": true' not in tool_evidence.lower()
                        )
                    },
                    tool_evidence
                )
                payload = tool_response(
                    "file_tree",
                    {"root": "outputs"},
                    "call_boundary_list_outputs"
                )
                payload["choices"][0]["delta"]["content"] = "INTERNAL_TOOL_PLAN_TWO"
            elif stage == 2 and regression_mode == "multi_turn_boundary":
                append_report(
                    stage,
                    {
                        "second_tool_result_reached_model": (
                            "outputs" in messages_text
                            and '"error":true' not in tool_evidence.lower()
                            and '"error": true' not in tool_evidence.lower()
                        )
                    },
                    tool_evidence
                )
                payload = final_response(
                    "## Final answer\n\n- first verified item\n- second verified item"
                )
            elif stage == 1:
                if regression_mode in ("object_write", "double_encoded_write"):
                    checks = {
                        "write_reached_real_tool": (
                            os.path.isfile(CONFIG["output_absolute_path"])
                            and '"error":true' not in tool_evidence.lower()
                            and '"error": true' not in tool_evidence.lower()
                        )
                    }
                    if regression_mode == "object_write":
                        token = "OBJECT_ARGUMENTS_E2E_COMPLETE"
                    else:
                        token = "DOUBLE_ENCODED_ARGUMENTS_E2E_COMPLETE"
                else:
                    checks = {
                        "invalid_arguments_reached_model": (
                            "missing required field" in messages_text.lower()
                            and not os.path.isfile(CONFIG["output_absolute_path"])
                        )
                    }
                    token = "EMPTY_TOOL_E2E_COMPLETE"
                append_report(stage, checks, tool_evidence)
                if regression_mode == "empty_write":
                    payload = tool_response(
                        "write_file",
                        {
                            "path": CONFIG["output_path"],
                            "content": "Recovery preview only.",
                            "dry_run": True
                        },
                        "call_recover_from_invalid_arguments"
                    )
                elif regression_mode in ("object_write", "double_encoded_write"):
                    payload = output_validation_response(
                        CONFIG["output_absolute_path"],
                        "call_validate_protocol_write"
                    )
                else:
                    payload = final_response(token)
            elif stage == 2 and regression_mode in ("object_write", "double_encoded_write"):
                validation_succeeded = output_validation_succeeded(
                    tool_evidence,
                    CONFIG["output_absolute_path"]
                )
                append_report(stage, {"output_validated": validation_succeeded}, tool_evidence)
                if regression_mode == "object_write":
                    payload = final_response("OBJECT_ARGUMENTS_E2E_COMPLETE")
                else:
                    payload = final_response("DOUBLE_ENCODED_ARGUMENTS_E2E_COMPLETE")
            elif stage == 2 and regression_mode == "empty_write":
                recovered = (
                    "write_file" in messages_text
                    and "dry_run" in messages_text
                    and '"error":true' not in tool_evidence.lower()
                    and '"error": true' not in tool_evidence.lower()
                )
                append_report(stage, {"invalid_call_recovered": recovered}, tool_evidence)
                payload = final_response("EMPTY_TOOL_E2E_COMPLETE")
            else:
                append_report(stage, {"unexpected_extra_request": False})
                payload = final_response("UNEXPECTED_EXTRA_REQUEST")
        elif CONFIG.get("skill_name") == "design-automations":
            if stage == 0:
                skill_advertised = skill_is_advertised(body.get("messages", []), CONFIG["skill_id"])
                source_root_hidden = CONFIG["skill_root"] not in messages_text
                append_report(stage, {
                    "user_prompt_preserved": CONFIG["prompt"] in messages_text,
                    "skill_available": "Skill" in host_tool_names and "skill_context" not in host_tool_names,
                    "orchestration_discovery_available": "tool_search" in host_tool_names,
                    "skill_advertised": skill_advertised,
                    "source_root_hidden": source_root_hidden
                }, body_text[:4000], observations={"root_visible": skill_advertised})
                payload = tool_response("Skill", {
                    "skill": CONFIG["skill_id"], "args": CONFIG["prompt"]
                }, "call_automation_skill")
            elif stage == 1:
                marker_loaded = CONFIG["skill_marker"].lower() in messages_text.lower()
                context_loaded = skill_context_loaded(messages_text, last_tool, CONFIG["skill_id"])
                append_report(stage, {
                    "skill_loaded": context_loaded,
                    "skill_marker_present": marker_loaded,
                    "native_persistence_contract_loaded": "nola_orchestrate" in messages_text
                }, body_text[:4000])
                automation_arguments = json.loads(
                    json.dumps(CONFIG["automation_arguments"]).replace(
                        "__MOCK_SOURCE_BASE__",
                        "http://127.0.0.1:" + str(self.server.server_address[1])
                    )
                )
                payload = tool_response(
                    "nola_orchestrate",
                    automation_arguments,
                    "call_create_native_automation"
                )
            elif stage == 2:
                orchestrated = (
                    "nola_orchestrate" in messages_text
                    and '"error":true' not in tool_evidence.lower()
                    and '"error": true' not in tool_evidence.lower()
                )
                append_report(stage, {"native_automation_created": orchestrated}, tool_evidence)
                payload = final_response("AUTOMATION_E2E_CREATED")
            elif (
                "call_automation_skill" not in messages_text
                and "call_execution_skill" not in messages_text
            ):
                append_report(stage, {
                    "stored_prompt_executed": "validated automation fixture" in messages_text,
                    "execution_skill_available": "Skill" in host_tool_names and "skill_context" not in host_tool_names,
                    "request_path": self.path,
                    "request_keys": sorted(body.keys())
                }, body_text[:4000])
                payload = tool_response("Skill", {
                    "skill": CONFIG["execution_skill_id"],
                    "args": "Run the validated automation fixture"
                }, "call_execution_skill")
            elif last_tool_name == "Skill" and "call_execution_skill" in messages_text:
                execution_skill_loaded = skill_context_loaded(
                    messages_text,
                    last_tool,
                    CONFIG["execution_skill_id"]
                )
                append_report(stage, {"execution_skill_loaded": execution_skill_loaded}, tool_evidence)
                payload = tool_response("write_file", {
                    "path": CONFIG["execution_output_path"],
                    "content": CONFIG["execution_output_content"]
                }, "call_write_automation_execution_output")
            elif last_tool_name == "write_file":
                output_written = (
                    os.path.isfile(CONFIG["execution_output_absolute_path"])
                    and "permission denied" not in tool_evidence.lower()
                    and '"error":true' not in tool_evidence.lower()
                    and '"error": true' not in tool_evidence.lower()
                )
                append_report(stage, {
                    "output_written": output_written,
                    "validation_tool_available": "bash" in host_tool_names
                }, body_text[:4000])
                payload = output_validation_response(
                    CONFIG["execution_output_absolute_path"],
                    "call_validate_automation_execution_output"
                )
            elif last_tool_name == "bash":
                validation_succeeded = output_validation_succeeded(
                    tool_evidence,
                    CONFIG["execution_output_absolute_path"]
                )
                append_report(stage, {"output_validated": validation_succeeded}, tool_evidence)
                payload = final_response("AUTOMATION_E2E_EXECUTION_COMPLETE")
            else:
                append_report(stage, {"unexpected_extra_request": False})
                payload = final_response("AUTOMATION_E2E_UNEXPECTED")
        elif CONFIG.get("skill_name") == "curate-knowledge-memory":
            if stage == 0:
                skill_advertised = skill_is_advertised(body.get("messages", []), CONFIG["skill_id"])
                source_root_hidden = CONFIG["skill_root"] not in messages_text
                append_report(stage, {
                    "user_prompt_preserved": CONFIG["prompt"] in messages_text,
                    "skill_available": "Skill" in host_tool_names and "skill_context" not in host_tool_names,
                    "memory_tool_discovery_available": "tool_search" in host_tool_names,
                    "skill_advertised": skill_advertised,
                    "source_root_hidden": source_root_hidden
                }, observations={"root_visible": skill_advertised})
                payload = tool_response("Skill", {
                    "skill": CONFIG["skill_id"], "args": CONFIG["prompt"]
                }, "call_memory_skill")
            elif stage == 1:
                marker_loaded = CONFIG["skill_marker"].lower() in messages_text.lower()
                context_loaded = skill_context_loaded(messages_text, last_tool, CONFIG["skill_id"])
                append_report(stage, {
                    "skill_loaded": context_loaded,
                    "skill_marker_present": marker_loaded
                }, tool_evidence)
                payload = tool_response("read_file", {"path": CONFIG["input_path"]}, "call_read_memory_source")
            elif stage == 2:
                input_read = CONFIG["input_token"] in messages_text
                ordinary_preference = (
                    "Classification: normal preference" in messages_text
                    and "Intended scope: global" in messages_text
                )
                append_report(stage, {
                    "input_read": input_read,
                    "ordinary_preference_classified": ordinary_preference
                }, tool_evidence)
                payload = tool_response("memory_save", {
                    "scope": "global",
                    "kind": "preference",
                    "content": CONFIG["input_token"],
                    "summary": "Concise release notes with verification",
                    "tags": ["release-notes", "verification", "e2e-7301"],
                    "source": "explicit user statement in the current conversation"
                }, "call_save_long_term_memory")
            elif stage == 3:
                memory_saved = (
                    "memory_saved: mem-" in messages_text
                    and CONFIG["input_token"] in messages_text
                    and '"error":true' not in tool_evidence.lower()
                    and '"error": true' not in tool_evidence.lower()
                )
                append_report(stage, {"memory_saved_with_stable_id": memory_saved}, tool_evidence)
                payload = tool_response("memory_search", {
                    "scope": "global",
                    "kind": "preference",
                    "query": "concise release notes verification",
                    "limit": 10
                }, "call_recall_long_term_memory")
            elif stage == 4:
                recalled = False
                recall_parse_error = ""
                try:
                    memory_result = next(
                        (
                            message for message in reversed(tool_messages)
                            if message.get("name") == "memory_search"
                            or '"memories"' in json.dumps(
                                message.get("content", {}),
                                ensure_ascii=False,
                                sort_keys=True
                            )
                        ),
                        {}
                    )
                    host_content = memory_result.get("content", {})
                    host_result = host_content if isinstance(host_content, dict) else json.loads(host_content)
                    search_content = host_result.get("text", {})
                    search_text = json.dumps(search_content, ensure_ascii=False) if isinstance(search_content, dict) else str(search_content)
                    recalled = (
                        '"count": 1' in search_text
                        and '"id": "mem-' in search_text
                        and '"scope": "global"' in search_text
                        and '"kind": "preference"' in search_text
                        and CONFIG["input_token"] in search_text
                        and '"source": "explicit user statement in the current conversation"' in search_text
                    )
                except (TypeError, ValueError, json.JSONDecodeError) as error:
                    recall_parse_error = repr(error)
                    recalled = False
                append_report(stage, {
                    "memory_recalled_from_persistent_store": recalled
                }, tool_evidence + " parse_error=" + recall_parse_error)
                payload = tool_response("write_file", {
                    "path": CONFIG["output_path"],
                    "content": CONFIG["output_content"]
                }, "call_write_memory_verification")
            elif stage == 5:
                output_written = (
                    CONFIG["output_path"] in messages_text
                    and "permission denied" not in tool_evidence.lower()
                    and '"error":true' not in tool_evidence.lower()
                    and '"error": true' not in tool_evidence.lower()
                )
                append_report(
                    stage,
                    {"output_written": output_written},
                    tool_evidence,
                    observations={
                        "host_tool_names": host_tool_names,
                        "messages_tail": body.get("messages", [])[-4:]
                    }
                )
                payload = output_validation_response(
                    CONFIG["artifact_output_absolute_path"],
                    "call_validate_memory_verification"
                )
            elif stage == 6:
                validation_succeeded = output_validation_succeeded(
                    tool_evidence,
                    CONFIG["artifact_output_absolute_path"]
                )
                append_report(stage, {"output_validated": validation_succeeded}, tool_evidence)
                payload = final_response(CONFIG["final_token"] + " stored and recalled")
            else:
                append_report(stage, {"unexpected_extra_request": False})
                payload = final_response(CONFIG["final_token"] + " unexpected extra request")
        elif CONFIG.get("skill_name") == "conduct-deep-research":
            origin = "http://127.0.0.1:" + str(self.server.server_address[1])
            if stage == 0:
                skill_advertised = skill_is_advertised(body.get("messages", []), CONFIG["skill_id"])
                source_root_hidden = CONFIG["skill_root"] not in messages_text
                append_report(stage, {
                    "user_prompt_preserved": CONFIG["prompt"] in messages_text,
                    "skill_available": "Skill" in host_tool_names and "skill_context" not in host_tool_names,
                    "skill_advertised": skill_advertised,
                    "source_root_hidden": source_root_hidden
                }, observations={"root_visible": skill_advertised})
                payload = tool_response("Skill", {
                    "skill": CONFIG["skill_id"], "args": CONFIG["prompt"]
                }, "call_research_skill")
            elif stage == 1:
                marker_loaded = CONFIG["skill_marker"].lower() in messages_text.lower()
                context_loaded = skill_context_loaded(messages_text, last_tool, CONFIG["skill_id"])
                append_report(stage, {"skill_loaded": context_loaded, "skill_marker_present": marker_loaded}, tool_evidence)
                payload = tool_response("read_file", {"path": CONFIG["input_path"]}, "call_read_research_question")
            elif stage == 2:
                input_read = CONFIG["input_token"] in messages_text
                append_report(stage, {"input_read": input_read}, tool_evidence)
                payload = tool_response("web_search", {
                    "query": CONFIG["input_token"], "limit": 20, "engine": "serper", "parse_pages": False
                }, "call_ranked_web_search")
            elif stage == 3:
                search_complete = (
                    CONFIG["input_token"] in messages_text
                    and "Nola Research Result 20" in messages_text
                    and "iana.org/help/example-domains" in messages_text
                    and "https://example.com/" in messages_text
                    and '"error":true' not in tool_evidence.lower()
                    and '"error": true' not in tool_evidence.lower()
                )
                append_report(stage, {"ranked_twenty_search_complete": search_complete}, tool_evidence)
                payload = tool_response("web_fetch", {"url": "https://www.iana.org/help/example-domains"}, "call_fetch_primary_source")
            elif stage == 4:
                primary_text = messages_text.lower()
                primary_opened = "iana" in primary_text and "example domain" in primary_text and "reserved" in primary_text
                append_report(stage, {"primary_source_opened": primary_opened}, tool_evidence)
                payload = tool_response("web_fetch", {"url": "https://example.com/"}, "call_fetch_independent_source")
            elif stage == 5:
                secondary_text = messages_text.lower()
                secondary_opened = "example domain" in secondary_text and "illustrative examples" in secondary_text
                append_report(stage, {"independent_source_opened": secondary_opened}, tool_evidence)
                content = CONFIG["output_content"]
                payload = tool_response("write_file", {"path": CONFIG["output_path"], "content": content}, "call_write_research_brief")
            elif stage == 6:
                output_written = (
                    CONFIG["output_path"] in messages_text
                    and "permission denied" not in tool_evidence.lower()
                    and '"error":true' not in tool_evidence.lower()
                    and '"error": true' not in tool_evidence.lower()
                )
                append_report(
                    stage,
                    {"output_written": output_written},
                    tool_evidence,
                    observations={
                        "host_tool_names": host_tool_names,
                        "messages_tail": body.get("messages", [])[-4:]
                    }
                )
                payload = output_validation_response(
                    CONFIG["artifact_output_absolute_path"],
                    "call_validate_research_brief"
                )
            elif stage == 7:
                validation_succeeded = output_validation_succeeded(
                    tool_evidence,
                    CONFIG["artifact_output_absolute_path"]
                )
                append_report(stage, {"output_validated": validation_succeeded}, tool_evidence)
                payload = final_response(CONFIG["final_token"] + " validated")
            else:
                append_report(stage, {"unexpected_extra_request": False})
                payload = final_response(CONFIG["final_token"] + " unexpected extra request")
        elif expect_failure and stage >= 1:
            marker_loaded = CONFIG["skill_marker"].lower() in messages_text.lower()
            if stage == 1:
                append_report(
                    stage,
                    {"disabled_skill_rejected": not marker_loaded},
                    tool_evidence
                )
            payload = final_response(CONFIG["final_token"] + " disabled skill rejected")
        elif stage == 0:
            skill_advertised = skill_is_advertised(body.get("messages", []), CONFIG["skill_id"])
            source_root_hidden = CONFIG["skill_root"] not in messages_text
            checks = {
                "user_prompt_preserved": CONFIG["prompt"] in messages_text,
                "skill_available": "Skill" in host_tool_names and "skill_context" not in host_tool_names,
                "skill_advertised": skill_advertised == (not expect_failure),
                "source_root_hidden": source_root_hidden
            }
            append_report(
                stage,
                checks,
                json.dumps({
                    "request_keys": sorted(body.keys()),
                    "host_tool_names": host_tool_names,
                    "tool_count": len(body.get("tools", []))
                }, sort_keys=True),
                observations={
                    "root_visible": skill_advertised,
                    "request_keys": sorted(body.keys()),
                    "host_tool_names": host_tool_names,
                    "tool_count": len(body.get("tools", []))
                }
            )
            payload = tool_response(
                "Skill",
                {"skill": CONFIG["skill_id"], "args": CONFIG["prompt"]},
                "call_skill"
            )
        elif stage == 1:
            marker_loaded = CONFIG["skill_marker"].lower() in messages_text.lower()
            context_loaded = skill_context_loaded(messages_text, last_tool, CONFIG["skill_id"])
            checks = {
                "skill_loaded": context_loaded,
                "skill_marker_present": marker_loaded
            }
            append_report(stage, checks, tool_evidence)
            if CONFIG.get("artifact_mode") == "word":
                    payload = tool_response(
                        "write_file",
                        {
                            "path": CONFIG["artifact_source_path"],
                            "content": CONFIG["artifact_source_content"]
                        },
                        "call_write_word_source"
                    )
            elif CONFIG.get("artifact_mode") == "ocr":
                    generator = shlex.quote(CONFIG["artifact_generator_absolute_path"])
                    source = shlex.quote(CONFIG["input_absolute_path"])
                    output = shlex.quote(CONFIG["artifact_output_absolute_path"])
                    command = "/usr/bin/swift " + generator + " --input " + source + " --output " + output
                    payload = tool_response(
                        "bash",
                        artifact_command_args(command, CONFIG["artifact_output_absolute_path"]),
                        "call_extract_image_text"
                    )
            else:
                payload = tool_response(
                    "read_file",
                    {"path": CONFIG["input_path"]},
                    "call_read_input"
                )
        elif stage == 2:
            artifact_mode = CONFIG.get("artifact_mode")
            if artifact_mode == "ocr":
                output_written = (
                    os.path.isfile(CONFIG["artifact_output_absolute_path"])
                    and os.path.getsize(CONFIG["artifact_output_absolute_path"]) > 500
                )
                input_read = CONFIG["input_token"] in messages_text
                append_report(stage, {"input_read": input_read, "output_written": output_written}, tool_evidence)
                payload = output_validation_response(
                    CONFIG["artifact_output_absolute_path"],
                    "call_validate_ocr_output"
                )
            elif artifact_mode == "word":
                source_written = (
                    CONFIG["artifact_source_path"] in messages_text
                    and '"error": true' not in tool_evidence.lower()
                    and '"error":true' not in tool_evidence.lower()
                )
                append_report(stage, {"artifact_source_written": source_written}, tool_evidence)
                generator = shlex.quote(CONFIG["artifact_generator_absolute_path"])
                source = shlex.quote(CONFIG["artifact_source_absolute_path"])
                output = shlex.quote(CONFIG["artifact_output_absolute_path"])
                command = "/usr/bin/python3 " + generator + " --spec " + source + " --output " + output
                payload = tool_response(
                    "bash",
                    {
                        "command": command,
                        "cwd": CONFIG["workspace_path"],
                        "operation_kind": "mutate_and_validate",
                        "validation_targets": [
                            CONFIG["artifact_source_path"],
                            CONFIG["artifact_output_absolute_path"]
                        ],
                        "output_artifacts": [{
                            "path": CONFIG["artifact_output_absolute_path"],
                            "operation": "create"
                        }]
                    },
                    "call_convert_word_document"
                )
            elif artifact_mode in ("excel", "powerpoint", "paper_powerpoint", "pdf"):
                input_read = CONFIG["input_token"] in messages_text
                append_report(stage, {"input_read": input_read}, tool_evidence)
                payload = tool_response(
                    "write_file",
                    {
                        "path": CONFIG["artifact_spec_path"],
                        "content": CONFIG["artifact_spec_content"]
                    },
                    "call_write_office_spec"
                )
            elif artifact_mode == "data_analysis":
                input_read = CONFIG["input_token"] in messages_text
                append_report(stage, {"input_read": input_read}, tool_evidence)
                generator = shlex.quote(CONFIG["artifact_generator_absolute_path"])
                source = shlex.quote(CONFIG["input_absolute_path"])
                output = shlex.quote(CONFIG["artifact_output_absolute_path"])
                command = "/usr/bin/python3 " + generator + " --input " + source + " --output " + output
                payload = tool_response(
                    "bash",
                    artifact_command_args(command, CONFIG["artifact_output_absolute_path"]),
                    "call_analyze_dataset"
                )
            elif artifact_mode == "schedule":
                input_read = (
                    "event-overnight-7301" in messages_text
                    and "proposal-clear" in messages_text
                    and "Asia/Shanghai" in messages_text
                )
                append_report(stage, {"input_read": input_read}, tool_evidence)
                generator = shlex.quote(CONFIG["artifact_generator_absolute_path"])
                source = shlex.quote(CONFIG["input_absolute_path"])
                output = shlex.quote(CONFIG["artifact_output_absolute_path"])
                command = "/usr/bin/python3 " + generator + " --input " + source + " --output " + output
                payload = tool_response(
                    "bash",
                    artifact_command_args(command, CONFIG["artifact_output_absolute_path"]),
                    "call_validate_schedule"
                )
            elif artifact_mode == "file_organization":
                input_read = (
                    "inbox/report.txt" in messages_text
                    and "organized/report.txt" in messages_text
                    and "inbox/shortcut" in messages_text
                )
                append_report(stage, {"input_read": input_read}, tool_evidence)
                generator = shlex.quote(CONFIG["artifact_generator_absolute_path"])
                source = shlex.quote(CONFIG["input_absolute_path"])
                manifest = shlex.quote(CONFIG["artifact_output_absolute_path"])
                command = "/usr/bin/python3 " + generator + " --plan " + source + " --manifest " + manifest
                payload = tool_response(
                    "bash",
                    artifact_command_args(command, CONFIG["artifact_output_absolute_path"]),
                    "call_plan_file_organization"
                )
            elif artifact_mode == "email":
                input_read = (
                    "thread-email-7301" in messages_text
                    and "qa-report-7302" in messages_text
                    and "FFT_results.xlsx" in messages_text
                )
                append_report(stage, {"input_read": input_read}, tool_evidence)
                generator = shlex.quote(CONFIG["artifact_generator_absolute_path"])
                source = shlex.quote(CONFIG["input_absolute_path"])
                output = shlex.quote(CONFIG["artifact_output_absolute_path"])
                command = "/usr/bin/python3 " + generator + " --input " + source + " --output " + output
                payload = tool_response(
                    "bash",
                    artifact_command_args(command, CONFIG["artifact_output_absolute_path"]),
                    "call_compose_email_draft"
                )
            elif artifact_mode == "figma":
                input_read = (
                    "FIGMA7301" in messages_text
                    and "42:100" in messages_text
                    and "42:101" in messages_text
                    and "color/accent" in messages_text
                )
                append_report(stage, {"input_read": input_read}, tool_evidence)
                generator = shlex.quote(CONFIG["artifact_generator_absolute_path"])
                source = shlex.quote(CONFIG["input_absolute_path"])
                output = shlex.quote(CONFIG["artifact_output_absolute_path"])
                command = "/usr/bin/python3 " + generator + " --input " + source + " --output " + output
                payload = tool_response(
                    "bash",
                    artifact_command_args(command, CONFIG["artifact_output_absolute_path"]),
                    "call_audit_figma_snapshot"
                )
            else:
                input_read = CONFIG["input_token"] in messages_text
                checks = {"input_read": input_read}
                if CONFIG.get("skill_name") == "review-documents":
                    checks.update({
                        "liability_evidence_read": "without limitation" in messages_text,
                        "control_terms_read": "Invoices are due Net 30" in messages_text and "30 days notice" in messages_text,
                        "section_anchor_read": "Section 4 - Liability" in messages_text
                    })
                elif CONFIG.get("skill_name") == "assist-meetings":
                    checks.update({
                        "explicit_decision_read": "Decision: launch the beta" in messages_text,
                        "accepted_commitment_read": "Mira: I will deliver the QA report" in messages_text,
                        "proposal_distinguished": "No decision was made" in messages_text,
                        "unassigned_action_read": "owner remains unassigned" in messages_text,
                        "timezone_read": "Asia/Shanghai" in messages_text
                    })
                append_report(stage, checks, tool_evidence)
                payload = tool_response(
                    "write_file",
                    {
                        "path": CONFIG["output_path"],
                        "content": CONFIG["output_content"]
                    },
                    "call_write_receipt"
                )
        elif stage == 3:
            artifact_mode = CONFIG.get("artifact_mode")
            if artifact_mode == "file_organization":
                planned = os.path.isfile(CONFIG["artifact_output_absolute_path"])
                append_report(stage, {
                    "dry_run_manifest_written": planned,
                    "output_written": planned
                }, tool_evidence)
                generator = shlex.quote(CONFIG["artifact_generator_absolute_path"])
                manifest = shlex.quote(CONFIG["artifact_output_absolute_path"])
                command = "/usr/bin/python3 " + generator + " --execute --manifest " + manifest
                payload = tool_response(
                    "bash",
                    organization_mutation_args(command),
                    "call_execute_file_organization"
                )
            elif artifact_mode in ("excel", "powerpoint", "paper_powerpoint", "pdf"):
                spec_written = (
                    CONFIG["artifact_spec_path"] in messages_text
                    and '"error": true' not in tool_evidence.lower()
                    and '"error":true' not in tool_evidence.lower()
                )
                append_report(stage, {"artifact_spec_written": spec_written}, tool_evidence)
                generator = shlex.quote(CONFIG["artifact_generator_absolute_path"])
                spec = shlex.quote(CONFIG["artifact_spec_absolute_path"])
                output = shlex.quote(CONFIG["artifact_output_absolute_path"])
                interpreter = "/usr/bin/swift " if artifact_mode == "pdf" else "/usr/bin/python3 "
                command = interpreter + generator + " --spec " + spec + " --output " + output
                if artifact_mode == "paper_powerpoint":
                    audit = shlex.quote(str(pathlib.Path(CONFIG["artifact_generator_absolute_path"]).with_name("audit_pptx_quality.py")))
                    command += " && /usr/bin/python3 " + audit + " " + output
                payload = tool_response(
                    "bash",
                    artifact_command_args(command, CONFIG["artifact_output_absolute_path"]),
                    "call_generate_office_artifact"
                )
            else:
                if artifact_mode in ("word", "data_analysis", "schedule", "email", "figma"):
                    minimum_size = 300 if artifact_mode == "email" else 1000
                    output_written = (
                        os.path.isfile(CONFIG["artifact_output_absolute_path"])
                        and os.path.getsize(CONFIG["artifact_output_absolute_path"]) > minimum_size
                    )
                else:
                    output_written = (
                        CONFIG["output_path"] in messages_text
                        and "permission denied" not in tool_evidence.lower()
                        and '"error": true' not in tool_evidence.lower()
                        and '"error":true' not in tool_evidence.lower()
                    )
                append_report(
                    stage,
                    {"output_written": output_written},
                    json.dumps({
                        "last_tool": last_tool,
                        "host_tool_names": host_tool_names,
                        "messages_tail": body.get("messages", [])[-4:]
                    }, ensure_ascii=False, sort_keys=True)
                )
                if artifact_mode in ("word", "data_analysis", "email", "figma", ""):
                    payload = output_validation_response(
                        CONFIG["artifact_output_absolute_path"],
                        "call_validate_generated_output"
                    )
                else:
                    payload = final_response(CONFIG["final_token"] + " validated")
        elif stage == 4:
            if CONFIG.get("artifact_mode") == "file_organization":
                executed = (
                    os.path.isfile(os.path.join(CONFIG["workspace_path"], "organized", "report (1).txt"))
                    and not os.path.exists(os.path.join(CONFIG["workspace_path"], "inbox", "report.txt"))
                )
                append_report(stage, {"approved_move_executed": executed}, tool_evidence)
                generator = shlex.quote(CONFIG["artifact_generator_absolute_path"])
                manifest = shlex.quote(CONFIG["artifact_output_absolute_path"])
                command = "/usr/bin/python3 " + generator + " --rollback --manifest " + manifest
                payload = tool_response(
                    "bash",
                    organization_mutation_args(command, rollback=True),
                    "call_rollback_file_organization"
                )
            else:
                artifact_mode = CONFIG.get("artifact_mode")
                if artifact_mode == "word":
                    output_written = (
                        os.path.isfile(CONFIG["artifact_output_absolute_path"])
                        and os.path.getsize(CONFIG["artifact_output_absolute_path"]) > 1000
                        and "validated" in messages_text
                        and "true" in messages_text.lower()
                    )
                elif artifact_mode in ("excel", "powerpoint", "paper_powerpoint", "pdf"):
                    output_written = (
                        os.path.isfile(CONFIG["artifact_output_absolute_path"])
                        and os.path.getsize(CONFIG["artifact_output_absolute_path"]) > 1000
                    )
                else:
                    output_written = output_validation_succeeded(
                        tool_evidence,
                        CONFIG["artifact_output_absolute_path"]
                    )
                append_report(
                    stage,
                    {"output_written": output_written},
                    json.dumps({
                        "last_tool": last_tool,
                        "host_tool_names": host_tool_names,
                        "messages_tail": body.get("messages", [])[-4:]
                    }, ensure_ascii=False, sort_keys=True)
                )
                if artifact_mode == "pdf":
                    payload = output_validation_response(
                        CONFIG["artifact_output_absolute_path"],
                        "call_validate_pdf_output"
                    )
                else:
                    payload = final_response(CONFIG["final_token"] + " validated")
        elif stage == 5:
            if CONFIG.get("artifact_mode") == "file_organization":
                rolled_back = (
                    os.path.isfile(os.path.join(CONFIG["workspace_path"], "inbox", "report.txt"))
                    and not os.path.exists(os.path.join(CONFIG["workspace_path"], "organized", "report (1).txt"))
                    and os.path.isfile(os.path.join(CONFIG["workspace_path"], "organized", "report.txt"))
                )
                append_report(stage, {"rollback_verified": rolled_back}, tool_evidence)
                payload = final_response(CONFIG["final_token"] + " validated")
            elif CONFIG.get("artifact_mode") == "pdf":
                validation_succeeded = output_validation_succeeded(
                    tool_evidence,
                    CONFIG["artifact_output_absolute_path"]
                )
                append_report(stage, {"output_validated": validation_succeeded}, tool_evidence)
                payload = final_response(CONFIG["final_token"] + " validated")
            else:
                append_report(
                    stage,
                    {"unexpected_extra_request": False},
                    body_text[-8000:],
                    observations={"host_tool_names": host_tool_names}
                )
                payload = final_response(CONFIG["final_token"] + " unexpected extra request")
        else:
            append_report(
                stage,
                {"unexpected_extra_request": False},
                body_text[-8000:],
                observations={"host_tool_names": host_tool_names}
            )
            payload = final_response(CONFIG["final_token"] + " unexpected extra request")

        finish_reason = payload.get("choices", [{}])[0].get("finish_reason")
        response_delta = payload.get("choices", [{}])[0].get("delta", {})
        response_content = response_delta.get("content")
        if (
            finish_reason == "stop"
            and isinstance(response_content, str)
            and response_content
            and synthesis_content is None
            and not is_synthesis_tool_surface(host_tool_names)
            and tool_messages
        ):
            with LOCK:
                LAST_PROVISIONAL_FINAL = response_content
            output_path = CONFIG.get("artifact_output_absolute_path") or CONFIG.get("execution_output_absolute_path")
            if response_content == "AUTOMATION_E2E_CREATED":
                output_path = CONFIG.get("automation_persistence_path")
            if output_path and not expect_failure and not regression_mode:
                # Every business final is provisional until a separate real
                # check supplies the versioned receipt. The following model
                # rounds settle Core's pending skill records before the final.
                POST_GENERATION_CHECK = output_path
                payload = output_validation_response(output_path, "call_post_generation_check_" + str(stage))

        self.send_payload(payload, body)

    def send_payload(self, payload, body):
        state = CURRENT_RUN_STATE
        state["stage"] = STAGE
        state["candidate"] = LAST_PROVISIONAL_FINAL
        state["validation"] = POST_GENERATION_CHECK
        state["responses"][CURRENT_REQUEST_KEY] = payload
        calls = payload.get("choices", [{}])[0].get("delta", {}).get("tool_calls", [])
        if calls:
            assert len(calls) == 1, "Fixture steps require one exact receipt before advancing"
            call = calls[0]
            arguments = call["function"].get("arguments", {})
            # Object and double-encoded argument fixtures intentionally cover
            # provider normalization. Inspect declarations without rewriting it.
            for _ in range(2):
                if isinstance(arguments, str):
                    arguments = json.loads(arguments)
            artifacts = arguments.get("output_artifacts", []) if isinstance(arguments, dict) else []
            state["pending"] = {"id": call["id"], "name": call["function"]["name"],
                                "artifacts": artifacts, "payload": payload}
        self.write_payload(payload, body)

    def write_payload(self, payload, body):
        if body.get("stream", True):
            encoded = (
                "data: " + json.dumps(payload, ensure_ascii=False, separators=(",", ":")) +
                "\n\ndata: [DONE]\n\n"
            ).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Connection", "close")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)
            self.wfile.flush()
            self.close_connection = True
        else:
            choice = payload["choices"][0]
            delta = choice.get("delta", {})
            message = {"role": "assistant", **delta}
            self.send_json({
                "choices": [{
                    "index": 0,
                    "message": message,
                    "finish_reason": choice.get("finish_reason", "stop")
                }]
            })

    def send_html(self, content, status=200):
        encoded = content.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(encoded)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(encoded)

    def send_json(self, payload, status=200):
        encoded = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Connection", "close")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)
        self.wfile.flush()
        self.close_connection = True

server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print("PORT " + str(server.server_address[1]), flush=True)
server.serve_forever()
"""#
}
