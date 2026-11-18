// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS automation save transactions, canonical automation/run/data-source/sync/prompt-template persistence, and data source registry suite.

import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacAutomationSaveTransactionTests: XCTestCase {
    func testOriginalExecutionSpecSurvivesPersistenceAndDetectsARewritingHostReceipt() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("automation-intent-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let support = root.appendingPathComponent("Automations")
        let port = try MacApplicationPort(databaseURL: support.appendingPathComponent(MacAppEnvironment.canonicalStoreFileName))
        let environment = MacAppEnvironment(applicationSupportRoot: root.appendingPathComponent("Support"), cachesRoot: root.appendingPathComponent("Caches"))
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let web = MacWebSourceStore(storeURL: root.appendingPathComponent("web.json"), registry: registry)
        func makeStore(_ client: any MacAutomationApplicationClient) throws -> MacAutomationStore {
            try MacAutomationStore(baseDirectory: support, synchronizeSystemJobs: false,
                appEnvironment: environment, authorizationResolver: { _ in true },
                activityStore: MacActivityStore(baseDirectory: root.appendingPathComponent("Activity")),
                completionNotifier: { _, _ in }, dataSourceRegistry: registry, webSourceStore: web,
                automationClient: client, skillCatalogClient: port)
        }
        let originalSpec = MacAutomationExecutionSpec(prompt: "  Remind me from this exact source.  ",
            executionScope: .localWorkspace(root: root.path), permissionProfile: "allow_all",
            modelSelection: "use_current_default_at_admission", contextRefs: [.init(kind: "personal_data", id: "platform:reminders")], skillRefs: [])
        let automation = MacAutomation(id: UUID(), name: "Exact source", prompt: originalSpec.prompt,
            frequency: .daily, weekday: 1, hour: 9, minute: 0, workspacePath: root.path, isEnabled: false,
            createdAt: .now, contextSources: [.personalData], executionSpec: originalSpec)
        let healthy = try makeStore(port)
        let verified = try healthy.upsertWithVerification(automation)
        XCTAssertNil(verified.verificationError)
        XCTAssertEqual(verified.committed.executionSpec, .agent(originalSpec))
        XCTAssertEqual(try makeStore(port).automations.first?.executionSpec, originalSpec)
        let rewritten = MacAutomationExecutionSpec(prompt: originalSpec.prompt,
            executionScope: originalSpec.executionScope, permissionProfile: "cautious",
            modelSelection: originalSpec.modelSelection, contextRefs: [], skillRefs: [])
        let malicious = try makeStore(RejectingAutomationWrites(reader: port, replacementSpec: rewritten))
        let receipt = try malicious.upsertWithVerification(automation)
        XCTAssertNotNil(receipt.verificationError, "A readback matching its rewritten receipt must not prove the original intent")
        XCTAssertEqual(receipt.submittedExecutionSpecDigest, verified.submittedExecutionSpecDigest)
        XCTAssertEqual(receipt.committed.executionSpec, .agent(rewritten), "Committed data remains observable after verification fails")
    }

    func testFrozenPlatformReferenceResolvesOneKindAndRejectsAggregateAliases() throws {
        XCTAssertEqual(try MacPlatformDataService.automationKind(for: "platform:reminders"), .reminders)
        XCTAssertEqual(try MacPlatformDataService.automationKind(for: "platform:calendar"), .calendar)
        XCTAssertThrowsError(try MacPlatformDataService.automationKind(for: "platform-context"))
        XCTAssertThrowsError(try MacPlatformDataService.automationKind(for: "personal_data"))
        XCTAssertThrowsError(try MacPlatformDataService.automationKind(for: "platform:notifications"))
    }

    func testPresentationFailureNeverAcknowledgesAutomationReceipt() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("automation-notification-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let support = root.appendingPathComponent("Automations")
        let port = try MacApplicationPort(databaseURL: support.appendingPathComponent(MacAppEnvironment.canonicalStoreFileName))
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let web = MacWebSourceStore(storeURL: root.appendingPathComponent("web.json"), registry: registry)
        let calls = AutomationACKCalls()
        let store = try MacAutomationStore(baseDirectory: support, synchronizeSystemJobs: false,
            appEnvironment: MacAppEnvironment(applicationSupportRoot: root.appendingPathComponent("Support"), cachesRoot: root.appendingPathComponent("Caches")),
            activityStore: MacActivityStore(baseDirectory: root.appendingPathComponent("Activity")),
            completionNotifier: { _, _ in throw CocoaError(.fileWriteOutOfSpace) }, dataSourceRegistry: registry, webSourceStore: web,
            automationClient: RejectingAutomationWrites(reader: port, ackRecorder: { calls.record() }), skillCatalogClient: port)
        // Transport fixture only; production authenticity remains the SQLite seal gate.
        let identity = MacAssistantJournalIdentity(source_store_epoch: "fixture", conversation_id: "conversation", run_id: "run", assistant_message_id: "assistant")
        let result = try JSONDecoder().decode(MacRunResult.self, from: JSONSerialization.data(withJSONObject: macRunResultFixture("blocked")))
        let digest = String(repeating: "a", count: 64)
        let seal = MacAssistantRunTerminalSeal(protocol_version: 1, admission: .init(identity: identity, sequence: "1", record_digest: digest), identity: identity,
            core_session_id: "core", transition_id: "transition", checkpoint_revision: "1", core_revision: "1", event_cursor: "1", result: result, result_digest: digest,
            completed_at_ms: 1, entry_count: "0", heads_count: "0", child_receipt_count: "0", manifest_digest: digest, seal_digest: digest)
        let receipt = MacAutomationExecutionReceipt(binding: .init(occurrenceID: "occurrence", definitionRevision: 1, conversationID: "conversation", submissionID: "submission", runID: "run", invocationConfigDigest: digest, boundAtMs: 0), terminal: seal, reconciledAtMs: 1, notificationID: "notification")
        let activity = MacActivityStore(baseDirectory: root.appendingPathComponent("IdentityActivity"))
        let automation = MacAutomation(id: UUID(), name: "Identity", prompt: "Fixture", frequency: .daily,
            weekday: 1, hour: 9, minute: 0, workspacePath: root.path, isEnabled: false, createdAt: .now)
        activity.recordAutomationRun(automation, startedAt: .now, receipt: receipt, logURL: nil)
        activity.recordAutomationRun(automation, startedAt: .now, receipt: receipt, logURL: nil)
        XCTAssertEqual(activity.entries.count, 1, "Notification identity deduplicates delivery independently of definition grouping")
        XCTAssertEqual(activity.entries.first?.automationIdentity?.definitionID, automation.id)
        XCTAssertEqual(activity.entries.first?.automationIdentity?.occurrenceID, "occurrence")
        XCTAssertEqual(activity.entries.first?.automationIdentity?.runID, "run")
        XCTAssertEqual(activity.entries.first?.automationIdentity?.notificationID, "notification")
        XCTAssertNil(activity.entries.first?.sourceID)
        await store.deliverNotification(name: "Automation", receipt: receipt)
        XCTAssertEqual(calls.count, 0, "Failed local presentation must leave the durable notification pending")
        XCTAssertNotNil(store.errorMessage)
    }

    func testFailedMutationsPreserveMemoryAndCanonicalRecordsAndRetryDoesNotDuplicate() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("automation-save-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let support = root.appendingPathComponent("Automations")
        let port = try MacApplicationPort(databaseURL: support.appendingPathComponent(MacAppEnvironment.canonicalStoreFileName))
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let web = MacWebSourceStore(storeURL: root.appendingPathComponent("web.json"), registry: registry)
        let environment = MacAppEnvironment(applicationSupportRoot: root.appendingPathComponent("Support"),
            cachesRoot: root.appendingPathComponent("Caches"))
        func store(_ client: any MacAutomationApplicationClient) throws -> MacAutomationStore {
            try MacAutomationStore(baseDirectory: support, launchAgentDirectory: root.appendingPathComponent("LaunchAgents"),
                synchronizeSystemJobs: false, appEnvironment: environment,
                authorizationResolver: { _ in true }, activityStore: MacActivityStore(baseDirectory: root.appendingPathComponent("Activity")),
                completionNotifier: { _, _ in }, dataSourceRegistry: registry, webSourceStore: web,
                automationClient: client, skillCatalogClient: port)
        }
        let healthy = try store(port)
        let skillID = try XCTUnwrap(MacBuiltInSkillID.allCases.first)
        let overrides = root.appendingPathComponent("SkillOverrides")
        try MacBuiltInSkillRegistry.setEnabled(true, for: skillID,
            overridesRootURL: overrides, store: port)
        XCTAssertTrue(try healthy.isSkillActive(skillID))
        try MacBuiltInSkillRegistry.setEnabled(false, for: skillID,
            overridesRootURL: overrides, store: port)
        XCTAssertFalse(try healthy.isSkillActive(skillID),
            "Admission must observe the injected catalog, not the shared application's Skill state")
        let original = MacAutomation(id: UUID(), name: "Original", prompt: "Review", frequency: .daily,
            weekday: 1, hour: 9, minute: 0, workspacePath: root.path, isEnabled: false, createdAt: Date())
        XCTAssertTrue(healthy.upsert(original))
        let failing = try store(RejectingAutomationWrites(reader: port))
        let baseline = try port.listAutomations()
        let initialMemory = failing.automations
        var edited = original
        edited.name = "Unsaved edit"
        XCTAssertFalse(failing.upsert(edited))
        XCTAssertEqual(failing.automations, initialMemory)
        var added = original
        added.id = UUID()
        added.name = "New draft"
        XCTAssertFalse(failing.upsert(added))
        XCTAssertEqual(failing.automations, initialMemory)
        failing.setEnabled(true, for: original)
        XCTAssertEqual(failing.automations, initialMemory)
        failing.delete(original)
        XCTAssertEqual(failing.automations, initialMemory)
        failing.replaceAll([added])
        XCTAssertEqual(failing.automations, initialMemory)
        XCTAssertNotNil(failing.errorMessage)
        XCTAssertEqual(try port.listAutomations(), baseline)
        let claim = MacClaimedAutomationRun(lease: .init(occurrenceID: "missing", definitionRevision: 1, workerID: "test", leaseToken: "lease"), identity: .init(conversationID: "conversation", submissionID: "submission", userMessageID: "user", assistantMessageID: "assistant"), executionSpec: .init(prompt: "Fixture", executionScope: .localWorkspace(root: root.path), permissionProfile: "cautious", modelSelection: "use_current_default_at_admission", contextRefs: [], skillRefs: []))
        XCTAssertNil(failing.reconcileRun(claim, observation: .exited(exitCode: 0)),
            "A storage error cannot publish successful completion")
        XCTAssertNil(healthy.reconcileRun(claim, observation: .exited(exitCode: 0)),
            "A rejected or stale claim is not a committed completion")

        let retry = try store(port)
        XCTAssertTrue(retry.upsert(added))
        XCTAssertTrue(retry.upsert(added))
        XCTAssertEqual(retry.automations.filter { $0.id == added.id }.count, 1)
        let reopened = try MacApplicationPort(databaseURL: healthy.persistenceURL)
        XCTAssertEqual(try reopened.listAutomations().filter { $0.id == added.id.uuidString.lowercased() }.count, 1)
        XCTAssertEqual(try reopened.listAutomations().count, 2)
    }
}

/// A deterministic application boundary failure, while reads use an isolated
/// real SQLite store. No production files, scheduler or permissions are touched.
nonisolated private struct RejectingAutomationWrites: MacAutomationApplicationClient {
    let reader: MacApplicationPort
    var replacementSpec: MacAutomationExecutionSpec? = nil
    var ackRecorder: (@Sendable () -> Void)? = nil
    private var failure: NSError { NSError(domain: "AutomationSaveFixture", code: 1) }
    @MainActor func listAutomations() throws -> [MacCanonicalAutomation] { try reader.listAutomations() }
    func reconcileAutomations(_ input: MacCanonicalReconcileAutomationsInput) throws -> [MacCanonicalAutomation] { throw failure }
    @MainActor func upsertAutomation(_ input: MacCanonicalAutomationInput) throws -> MacCanonicalAutomation {
        guard let replacementSpec else { throw failure }
        return try reader.upsertAutomation(.init(id: input.id, name: input.name, scheduleFormat: input.scheduleFormat,
            enabled: input.enabled, revision: input.revision, executionSpec: .agent(replacementSpec), metadataJSON: input.metadataJSON,
            createdAtMs: input.createdAtMs, updatedAtMs: input.updatedAtMs, deletedAtMs: input.deletedAtMs))
    }
    func enqueueAutomationRun(_ input: MacCanonicalAutomationRun) throws -> MacCanonicalAutomationRun { throw failure }
    func claimAutomationRun(id: String, workerID: String, leaseToken: String, nowMs: Int64, leaseUntilMs: Int64) throws -> Bool { throw failure }
    func heartbeatAutomationRun(id: String, leaseToken: String, nowMs: Int64, leaseUntilMs: Int64) throws -> Bool { throw failure }
    func observeAutomationExecution(occurrenceID: String) throws -> MacAutomationExecutionObservation { throw failure }
    func listPendingAutomationExecutions(afterOccurrenceID: String?, limit: Int) throws -> [String] { throw failure }
    func reconcileAutomationExecution(occurrenceID: String, reconciledAtMs: Int64) throws -> MacAutomationExecutionReceipt? { throw failure }
    func observeAutomationWorker(lease: MacAutomationExecutionLease, observation: MacAutomationWorkerObservation, observedAtMs: Int64) throws -> Bool { throw failure }
    func ackAutomationNotification(occurrenceID: String, notificationID: String, deliveredAtMs: Int64) throws -> Bool { ackRecorder?(); throw failure }
    func loadAutomationRun(id: String) throws -> MacCanonicalAutomationRun? { throw failure }
    func listAutomationRuns(automationID: String?, limit: Int) throws -> [MacCanonicalAutomationRun] { throw failure }
}

nonisolated private final class AutomationACKCalls: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    var count: Int { lock.withLock { value } }
    func record() { lock.withLock { value += 1 } }
}

final class MacCanonicalAutomationEndToEndTests: XCTestCase {
    func testAutomationDefinitionSurvivesColdReopenAndReconcilesDeletion() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-automation-e2e-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("State/nola.db")
        let createdAt: Int64 = 1_800_000_000_000
        let id = UUID().uuidString.lowercased()
        let metadata = #"{"id":"automation-e2e","frequency":"daily","hour":9,"minute":30}"#
        let spec = MacAutomationExecutionSpec(prompt: "Review", executionScope: .localWorkspace(root: root.path), permissionProfile: "cautious", modelSelection: "use_current_default_at_admission", contextRefs: [], skillRefs: [])

        do {
            let store = try MacApplicationPort(databaseURL: database)
            let inserted = try store.upsertAutomation(
                MacCanonicalAutomationInput(
                    id: id,
                    name: "Daily review",
                    scheduleFormat: "nola.mac.automation.v1",
                    enabled: true,
                    revision: 0,
                    executionSpec: .agent(spec),
                    metadataJSON: metadata,
                    createdAtMs: createdAt,
                    updatedAtMs: createdAt,
                    deletedAtMs: nil
                )
            )
            XCTAssertEqual(inserted.revision, 1)

            let unchanged = try store.upsertAutomation(
                MacCanonicalAutomationInput(
                    id: id,
                    name: "Daily review",
                    scheduleFormat: "nola.mac.automation.v1",
                    enabled: true,
                    revision: inserted.revision,
                    executionSpec: .agent(spec),
                    metadataJSON: metadata,
                    createdAtMs: createdAt,
                    updatedAtMs: createdAt + 1,
                    deletedAtMs: nil
                )
            )
            XCTAssertEqual(unchanged.revision, 1)
        }

        do {
            let reopened = try MacApplicationPort(databaseURL: database)
            let restored = try XCTUnwrap(reopened.listAutomations().first)
            XCTAssertEqual(restored.id, id)
            XCTAssertEqual(restored.name, "Daily review")
            XCTAssertEqual(restored.metadataJSON, metadata)

            _ = try reopened.upsertAutomation(MacCanonicalAutomationInput(
                id: "web-fixture", name: "Web task", scheduleFormat: "script",
                enabled: false, revision: 0, executionSpec: .agent(spec), metadataJSON: #"{"id":"web-fixture","format":"script"}"#,
                createdAtMs: createdAt, updatedAtMs: createdAt, deletedAtMs: nil
            ))

            let remaining = try reopened.reconcileAutomations(
                MacCanonicalReconcileAutomationsInput(
                    inputs: [],
                    reconciledAtMs: createdAt + 2
                )
            )
            XCTAssertTrue(remaining.isEmpty)
        }

        let finalReopen = try MacApplicationPort(databaseURL: database)
        XCTAssertEqual(try finalReopen.listAutomations().map(\.id), ["web-fixture"], "Mac reconciliation must not delete Web tasks")
    }
}

@MainActor
final class MacCanonicalAutomationRunEndToEndTests: XCTestCase {
    func testLeaseOwnershipAndUnboundExitCannotManufactureResultAfterColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-automation-run-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let databaseURL = root.appendingPathComponent("nola.db")
        let now: Int64 = 1_800_000_000_000
        let input = MacCanonicalAutomationRun(
            id: "run-first",
            automationID: "automation-first",
            definitionRevision: 1,
            occurrenceKey: "scheduled:1800000000",
            trigger: "schedule",
            status: "queued",
            attempt: 0,
            workerID: nil,
            leaseToken: nil,
            leaseUntilMs: nil,
            scheduledForMs: now,
            startedAtMs: nil,
            finishedAtMs: nil,
            nextRetryAtMs: nil,
            resultJSON: nil,
            errorJSON: nil,
            createdAtMs: now,
            updatedAtMs: now
        )
        let encoded = try JSONEncoder().encode(input)
        XCTAssertEqual(try JSONDecoder().decode(MacCanonicalAutomationRun.self, from: encoded), input)
        var missingRevision = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        missingRevision.removeValue(forKey: "definition_revision")
        XCTAssertThrowsError(try JSONDecoder().decode(MacCanonicalAutomationRun.self,
            from: JSONSerialization.data(withJSONObject: missingRevision)))

        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            let definition = try store.upsertAutomation(.init(id: "automation-first", name: "Fixture", scheduleFormat: "nola.mac.automation.v1", enabled: true, revision: 0,
                executionSpec: .agent(.init(prompt: "Fixture", executionScope: .localWorkspace(root: root.path), permissionProfile: "cautious", modelSelection: "use_current_default_at_admission", contextRefs: [], skillRefs: [])),
                metadataJSON: "{}", createdAtMs: now, updatedAtMs: now, deletedAtMs: nil))
            let first = try store.enqueueAutomationRun(input)
            XCTAssertEqual(first.definitionRevision, definition.revision)
            let duplicate = try store.enqueueAutomationRun(MacCanonicalAutomationRun(
                id: "run-duplicate",
                automationID: input.automationID,
                definitionRevision: input.definitionRevision,
                occurrenceKey: input.occurrenceKey,
                trigger: input.trigger,
                status: input.status,
                attempt: 0,
                workerID: nil,
                leaseToken: nil,
                leaseUntilMs: nil,
                scheduledForMs: now,
                startedAtMs: nil,
                finishedAtMs: nil,
                nextRetryAtMs: nil,
                resultJSON: nil,
                errorJSON: nil,
                createdAtMs: now,
                updatedAtMs: now
            ))
            XCTAssertEqual(first.id, duplicate.id)
            XCTAssertTrue(try store.claimAutomationRun(
                id: first.id,
                workerID: "mac-test-a",
                leaseToken: "lease-a",
                nowMs: now,
                leaseUntilMs: now + 60_000
            ))
            XCTAssertFalse(try store.claimAutomationRun(
                id: first.id,
                workerID: "mac-test-b",
                leaseToken: "lease-b",
                nowMs: now + 1,
                leaseUntilMs: now + 60_001
            ))
            let observed = try store.observeAutomationExecution(occurrenceID: first.id)
            XCTAssertNil(observed.binding)
            XCTAssertNil(observed.terminal)
            XCTAssertTrue(try store.observeAutomationWorker(lease: .init(occurrenceID: first.id, definitionRevision: definition.revision, workerID: "mac-test-a", leaseToken: "lease-a"), observation: .exited(exitCode: 0), observedAtMs: now + 2))
            XCTAssertNil(try store.reconcileAutomationExecution(occurrenceID: first.id, reconciledAtMs: now + 2),
                "A claimed occurrence without a bound Run cannot manufacture a successful result")
        }

        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        let persisted = try XCTUnwrap(reopened.loadAutomationRun(id: "run-first"))
        XCTAssertNil(persisted.resultJSON)
        XCTAssertEqual(persisted.definitionRevision, input.definitionRevision)
        let observation = try reopened.observeAutomationExecution(occurrenceID: persisted.id)
        XCTAssertNil(observation.receipt)
        XCTAssertNil(observation.terminal)
        XCTAssertFalse(try reopened.claimAutomationRun(id: persisted.id, workerID: "replacement", leaseToken: "replacement",
            nowMs: now + 70_000, leaseUntilMs: now + 90_000), "Lease expiry alone cannot replay an uncertain occurrence")
        let failure = MacAutomationWorkerObservation.launchFailed(detail: "Fixture dependency platform:reminders failed before admission")
        XCTAssertTrue(try reopened.observeAutomationWorker(lease: .init(occurrenceID: persisted.id, definitionRevision: 1,
            workerID: "mac-test-a", leaseToken: "lease-a"), observation: failure, observedAtMs: now + 70_001))
        let failed = try reopened.observeAutomationExecution(occurrenceID: persisted.id)
        XCTAssertEqual(failed.phase, .launchFailed)
        XCTAssertEqual(failed.latestWorkerObservation, failure)
        XCTAssertNil(failed.binding)
        XCTAssertNil(failed.receipt, "Startup failure does not fabricate an admitted Run seal")
        XCTAssertEqual(try reopened.loadAutomationRun(id: persisted.id)?.status, "launch_failed")
        XCTAssertEqual(try reopened.listAutomationRuns(automationID: "automation-first").count, 1)
    }
}

final class MacCanonicalDataSourceEndToEndTests: XCTestCase {
    func testValidatedDefinitionOverrideAndUsageSurviveColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-data-source-e2e-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let databaseURL = root.appendingPathComponent("nola.db")
        let sourceID = "web:fixture"
        let now = Int64(Date().timeIntervalSince1970 * 1_000)

        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            XCTAssertTrue(try store.createConversation(MacCanonicalCreateConversationInput(
                id: "conversation-a", title: "Data source consumer", settingsJSON: nil,
                createdAtMs: now, updatedAtMs: now
            )).created)
            let contract = MacDataSourceContract(
                sourceID: sourceID,
                connectorID: "mac.test.http",
                connectorVersion: 1,
                readerDescription: "Bounded fixture read",
                readRecipe: MacDataSourceReadRecipe(
                    method: .httpGet,
                    operationID: "mac.test.http.read",
                    configurationKeys: ["url"],
                    outputSchemaJSON: #"{"type":"object","properties":{"title":{"type":"string"}}}"#
                ),
                schemaFields: ["title"],
                redactedSample: "fixture",
                recordCount: 1,
                maximumAgeMinutes: 60,
                validationState: .verified,
                validatedAt: .now,
                observedAt: .now,
                errorMessage: nil
            )
            let draft = try store.createDataSourceDraft(MacCanonicalDataSourceDraft(
                id: sourceID,
                name: "Fixture",
                kind: .web,
                configuration: MacCanonicalDataSourceConfiguration(
                    connectorID: "mac.test.http",
                    locationKind: .httpURL,
                    location: "https://example.test/data.json",
                    parameters: ["metadataJSON": .string(#"{"version":1}"#)],
                    secretReferences: []
                ),
                accessPolicy: MacDataSourceAccessPolicy.conversationOptIn.rawValue,
                authorization: MacCanonicalDataSourceAuthorization(
                    state: .notRequired,
                    capability: nil,
                    observedAtMs: now
                ),
                freshnessPolicy: MacCanonicalDataSourceFreshnessPolicy(
                    maximumAgeMinutes: 60,
                    allowStaleFallback: false
                ),
                capabilities: ["read", "context"],
                privacy: "cloud_reference",
                createdAtMs: now,
                updatedAtMs: now
            ))
            let pending = try store.requestDataSourceValidation(
                id: sourceID,
                expectedRevision: draft.revision,
                requestedAtMs: now + 1
            )
            let validated = try store.recordDataSourceValidationSuccess(
                id: sourceID,
                validationToken: try XCTUnwrap(pending.validation.validationToken),
                contract: contract,
                recordedAtMs: now + 2
            )
            _ = try store.setDataSourceDefaultEnabled(
                id: sourceID,
                expectedRevision: validated.revision,
                enabled: true,
                updatedAtMs: now + 3
            )
            XCTAssertThrowsError(try store.setDataSourceOverride(
                conversationID: "missing-conversation", sourceID: sourceID,
                value: .enabled, updatedAtMs: now + 4
            ))
            XCTAssertNil(try store.loadConversation("missing-conversation"),
                "An override cannot implicitly create its conversation owner")
            XCTAssertTrue(try store.setDataSourceOverride(
                conversationID: "conversation-a",
                sourceID: sourceID,
                value: .enabled,
                updatedAtMs: now + 4
            ))
            XCTAssertTrue(try store.appendDataSourceUsage(MacCanonicalDataSourceUsageInput(
                id: UUID().uuidString,
                sourceID: sourceID,
                sourceTitle: "Fixture",
                consumer: "chat",
                purpose: "answer",
                conversationID: "conversation-a",
                disclosedToCloud: true,
                metadata: [:],
                usedAtMs: now + 5
            )))
        }

        do {
            let reopened = try MacApplicationPort(databaseURL: databaseURL)
            let record = try XCTUnwrap(reopened.listDataSources().first { $0.id == sourceID })
            XCTAssertEqual(record.name, "Fixture")
            XCTAssertEqual(record.configuration.location, "https://example.test/data.json")
            XCTAssertTrue(record.defaultEnabled)
            XCTAssertEqual(
                try reopened.dataSourceOverride(
                    conversationID: "conversation-a",
                    sourceID: sourceID
                ),
                .enabled
            )
            let usage = try XCTUnwrap(
                reopened.listDataSourceUsage().first { $0.sourceID == sourceID }
            )
            XCTAssertEqual(usage.conversationID, "conversation-a")
            XCTAssertTrue(usage.disclosedToCloud)
        }
    }
}

final class MacCanonicalSyncEndToEndTests: XCTestCase {
    func testRemoteReplicaPageAppliesThroughRealCoreABIAndSurvivesColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-remote-sync-e2e-\(UUID().uuidString)", isDirectory: true)
        let sourceURL = root.appendingPathComponent("source.db")
        let targetURL = root.appendingPathComponent("target.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let pending: MacCanonicalPendingSyncChange
        do {
            let source = try MacApplicationPort(databaseURL: sourceURL)
            _ = try source.putProductSetting(
                key: "assistant.memory.enabled",
                valueJSON: "true",
                updatedAtMs: 100
            )
            pending = try XCTUnwrap(source.pendingSyncChanges().first)
        }

        do {
            let target = try MacApplicationPort(databaseURL: targetURL)
            let result = try target.applyRemoteSyncPage(
                replicaID: "account-replica",
                changes: [.init(
                    cursor: 1,
                    operationID: pending.operationID,
                    entityType: pending.entityType,
                    entityID: pending.entityID,
                    revision: pending.revision,
                    deleted: pending.deleted,
                    payloadJSON: pending.payloadJSON,
                    createdAtMs: pending.createdAtMs
                )],
                nextCursor: 1,
                appliedAtMs: 200
            )
            XCTAssertEqual(result.applied, 1)
            XCTAssertEqual(result.skipped, 0)
        }

        let reopened = try MacApplicationPort(databaseURL: targetURL)
        XCTAssertEqual(try reopened.productSetting(key: "assistant.memory.enabled")?.valueJSON, "true")
        XCTAssertEqual(try reopened.syncCursor(replicaID: "account-replica")?.cursor, 1)
        XCTAssertTrue(try reopened.pendingSyncChanges().isEmpty)
    }

    func testProductSettingOutboxAcknowledgementTombstoneAndCursorSurviveColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-sync-e2e-\(UUID().uuidString)", isDirectory: true)
        let databaseURL = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let operationID: String
        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            _ = try store.putProductSetting(
                key: "assistant.memory.enabled",
                valueJSON: #"{"enabled":true}"#,
                updatedAtMs: 10
            )
            let pending = try store.pendingSyncChanges()
            XCTAssertEqual(pending.count, 1)
            XCTAssertEqual(pending[0].entityType, "product_setting")
            operationID = pending[0].operationID
        }

        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            XCTAssertEqual(try store.pendingSyncChanges()[0].operationID, operationID)
            XCTAssertTrue(try store.resolveSyncChange(
                operationID: operationID,
                status: "acknowledged",
                remoteCursor: 7,
                resolvedAtMs: 20
            ))
            XCTAssertEqual(try store.saveSyncCursor(replicaID: "primary", cursor: 7, updatedAtMs: 20).cursor, 7)
            XCTAssertTrue(try store.deleteProductSetting(key: "assistant.memory.enabled"))
        }

        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        XCTAssertEqual(try reopened.syncCursor(replicaID: "primary")?.cursor, 7)
        let tombstones = try reopened.pendingSyncChanges()
        XCTAssertEqual(tombstones.count, 1)
        XCTAssertTrue(tombstones[0].deleted)
        XCTAssertEqual(tombstones[0].revision, 2)
    }
}

final class MacPromptTemplateCanonicalEndToEndTests: XCTestCase {
    func testPromptTemplateSnapshotSurvivesRealCoreReopenWithoutLegacyDefaults() throws {
        let database = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-prompts-\(UUID().uuidString).sqlite")
        defer { try? FileManager.default.removeItem(at: database) }

        UserDefaults.standard.set(Data("legacy".utf8), forKey: "promptTemplates")
        UserDefaults.standard.set(UUID().uuidString, forKey: "nola.mac.prompt.defaultTemplateID")
        defer {
            UserDefaults.standard.removeObject(forKey: "promptTemplates")
            UserDefaults.standard.removeObject(forKey: "nola.mac.prompt.defaultTemplateID")
        }

        let template = PromptTemplate(name: "Canonical", content: "body", description: "test")
        let snapshot = PromptTemplateSnapshot(customTemplates: [template], selectedTemplateID: template.id)
        do {
            let store = try MacApplicationPort(databaseURL: database)
            let data = try JSONEncoder().encode(snapshot)
            _ = try store.putProductSetting(
                key: MacProductSettingKeys.promptTemplates,
                valueJSON: String(decoding: data, as: UTF8.self),
                updatedAtMs: 1
            )
        }

        let reopened = try MacApplicationPort(databaseURL: database)
        let setting = try XCTUnwrap(reopened.productSetting(key: MacProductSettingKeys.promptTemplates))
        let restored = try JSONDecoder().decode(
            PromptTemplateSnapshot.self,
            from: try XCTUnwrap(setting.valueJSON.data(using: .utf8))
        )
        XCTAssertEqual(restored, snapshot)
    }
}

@MainActor
final class MacDataSourceRegistryTests: XCTestCase {
    func testDisplayOverrideSnapshotIsConversationScopedAndDoesNotReplaceRuntimeReads() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-data-display-\(UUID().uuidString)", isDirectory: true)
        let storeURL = directory.appendingPathComponent("registry.json")
        defer { try? FileManager.default.removeItem(at: directory) }
        try createConsumerConversation(storeURL: storeURL, id: "a")
        try createConsumerConversation(storeURL: storeURL, id: "b")
        let registry = MacDataSourceRegistry(storeURL: storeURL)
        let source = MacDataSourceRegistry.platformSourceID("calendar")
        registry.recordValidation(sourceID: source, connectorID: "eventkit.events",
            readerDescription: "fixture", schemaFields: ["title"], redactedSample: "{}",
            recordCount: 1, maximumAgeMinutes: 15, observedAt: .now)
        registry.setPolicy(.conversationOptIn, for: source)
        let revision = registry.overrideDisplayRevision
        registry.setConversationOverride(true, sourceID: source, conversationID: "a")
        XCTAssertGreaterThan(registry.overrideDisplayRevision, revision)
        let first = try await registry.loadOverridesForDisplay(conversationID: "a", sourceIDs: [source, source])
        let second = try await registry.loadOverridesForDisplay(conversationID: "b", sourceIDs: [source])
        XCTAssertEqual(first, [source: true])
        XCTAssertTrue(second.isEmpty)
        XCTAssertEqual(registry.displayResolution(sourceID: source, override: first[source])?.enabled, true)
        registry.setConversationOverride(false, sourceID: source, conversationID: "a")
        XCTAssertFalse(registry.isEnabled(sourceID: source, conversationID: "a"),
            "An old display snapshot must not authorize runtime access")
        let refreshed = try await registry.loadOverridesForDisplay(conversationID: "a", sourceIDs: [source])
        XCTAssertEqual(refreshed, [source: false])
        registry.setPolicy(.blocked, for: source)
        XCTAssertEqual(registry.displayResolution(sourceID: source, override: first[source])?.enabled, false)
        registry.setConversationOverride(nil, sourceID: source, conversationID: "a")
        let inherited = try await registry.loadOverridesForDisplay(conversationID: "a", sourceIDs: [source])
        XCTAssertTrue(inherited.isEmpty)
    }

    private func createConsumerConversation(storeURL: URL, id: String) throws {
        let store = try MacApplicationPort(databaseURL: storeURL.deletingLastPathComponent()
            .appendingPathComponent(MacAppEnvironment.canonicalStoreFileName))
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        XCTAssertTrue(try store.createConversation(MacCanonicalCreateConversationInput(
            id: id, title: "Data source consumer", settingsJSON: nil,
            createdAtMs: now, updatedAtMs: now
        )).created)
    }

    func testValidatedContractPolicyConversationOverrideAndUsageSurviveRestart() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-data-source-registry-\(UUID().uuidString)", isDirectory: true)
        let storeURL = directory.appendingPathComponent("registry.json")
        defer { try? FileManager.default.removeItem(at: directory) }

        let sourceID = MacDataSourceRegistry.platformSourceID("calendar")
        try createConsumerConversation(storeURL: storeURL, id: "conversation-a")
        try createConsumerConversation(storeURL: storeURL, id: "conversation-b")
        let registry = MacDataSourceRegistry(storeURL: storeURL)
        registry.recordValidation(
            sourceID: sourceID,
            connectorID: "eventkit.events",
            readerDescription: "EKEventStore.events(next: 30 days)",
            schemaFields: ["title", "startDate"],
            redactedSample: #"{"title":"<redacted>"}"#,
            recordCount: 3,
            maximumAgeMinutes: 15,
            observedAt: .now
        )
        registry.setPolicy(.conversationOptIn, for: sourceID)
        registry.setConversationOverride(true, sourceID: sourceID, conversationID: "conversation-a")
        XCTAssertNil(registry.persistenceError)
        registry.recordUsage(
            sourceID: sourceID,
            sourceTitle: "Calendar",
            consumer: "chat",
            purpose: "context",
            conversationID: "conversation-a",
            disclosedToCloud: true
        )

        XCTAssertFalse(registry.isEnabled(sourceID: sourceID, conversationID: "conversation-b"))
        XCTAssertTrue(registry.isEnabled(sourceID: sourceID, conversationID: "conversation-a"))

        let restored = MacDataSourceRegistry(storeURL: storeURL)
        XCTAssertEqual(restored.contract(for: sourceID)?.recordCount, 3)
        XCTAssertEqual(restored.contract(for: sourceID)?.schemaFields, ["title", "startDate"])
        XCTAssertEqual(restored.policy(for: sourceID), .conversationOptIn)
        XCTAssertTrue(restored.isEnabled(sourceID: sourceID, conversationID: "conversation-a"))
        XCTAssertEqual(restored.usageRecords.first?.conversationID, "conversation-a")

        let canonical = try MacApplicationPort(
            databaseURL: storeURL.deletingLastPathComponent().appendingPathComponent(
                MacAppEnvironment.canonicalStoreFileName
            )
        )
        let stored = try XCTUnwrap(canonical.listDataSources().first { $0.id == sourceID })
        XCTAssertEqual(stored.kind, .platform)
        XCTAssertEqual(stored.configuration.location, sourceID)
    }

    func testBlockedPolicyCannotBecomeEnabledWithoutExplicitConversationOverride() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-data-source-policy-\(UUID().uuidString)", isDirectory: true)
        let storeURL = directory.appendingPathComponent("registry.json")
        defer { try? FileManager.default.removeItem(at: directory) }
        try createConsumerConversation(storeURL: storeURL, id: "conversation")
        let registry = MacDataSourceRegistry(storeURL: storeURL)
        let sourceID = MacDataSourceRegistry.platformSourceID("contacts")

        registry.setPolicy(.blocked, for: sourceID)
        XCTAssertFalse(registry.isEnabled(sourceID: sourceID, conversationID: nil))
        XCTAssertFalse(registry.isEnabled(sourceID: sourceID, conversationID: "conversation"))

        registry.setConversationOverride(true, sourceID: sourceID, conversationID: "conversation")
        XCTAssertNil(registry.persistenceError, "Blocked access must not be confused with a failed override write")
        XCTAssertFalse(registry.isEnabled(sourceID: sourceID, conversationID: "conversation"))
    }

    func testFailedRevalidationPreservesKnownSchemaAndMarksContractUnusable() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-data-source-failure-\(UUID().uuidString)", isDirectory: true)
        let storeURL = directory.appendingPathComponent("registry.json")
        defer { try? FileManager.default.removeItem(at: directory) }
        let registry = MacDataSourceRegistry(storeURL: storeURL)
        let sourceID = MacDataSourceRegistry.platformSourceID("reminders")

        registry.recordValidation(
            sourceID: sourceID,
            connectorID: "eventkit.reminders",
            readerDescription: "fetch reminders",
            schemaFields: ["title", "dueDate"],
            redactedSample: #"{"title":"<redacted>"}"#,
            recordCount: 0,
            maximumAgeMinutes: 15,
            observedAt: .now
        )
        registry.recordValidationFailure(
            sourceID: sourceID,
            connectorID: "eventkit.reminders",
            readerDescription: "fetch reminders",
            error: "permission revoked"
        )

        let contract = try XCTUnwrap(registry.contract(for: sourceID))
        XCTAssertEqual(contract.validationState, .failed)
        XCTAssertFalse(contract.isUsable)
        XCTAssertEqual(contract.schemaFields, ["title", "dueDate"])
        XCTAssertEqual(contract.errorMessage, "permission revoked")
    }
}
