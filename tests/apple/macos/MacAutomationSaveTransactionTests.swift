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
