// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated iOS data source batch/status/boundary, Health dashboard, hybrid capabilities, prompt templates, scheduler/background wake, URL history, user tasks, log paths, software updates, and Office document extractor suite.

import Foundation
import HealthKit
import Testing
import UIKit
import XCTest
@testable import Nola

@MainActor
final class HealthDashboardStoreTests: XCTestCase {
    func testInitialHealthAuthorizationWaitIsNotPresentedAsRefreshFailure() async {
        let store = HealthDashboardStore {
            throw NSError(domain: HKErrorDomain, code: HKError.Code.errorAuthorizationNotDetermined.rawValue)
        }
        await store.load()
        XCTAssertNil(store.snapshot)
        XCTAssertNil(store.errorMessage)
        XCTAssertFalse(store.isLoading)
    }

    func testDeniedHealthAuthorizationStillSurfacesFailure() async {
        let store = HealthDashboardStore {
            throw NSError(domain: HKErrorDomain, code: HKError.Code.errorAuthorizationDenied.rawValue)
        }
        await store.load()
        XCTAssertNil(store.snapshot)
        XCTAssertNotNil(store.errorMessage)
        XCTAssertFalse(store.isLoading)
    }

    func testConcurrentLoadsWaitForTheSameSnapshotRequest() async {
        let gate = HealthDashboardTestGate()
        let completion = HealthDashboardTestFlag()
        var loadCount = 0
        let store = HealthDashboardStore {
            loadCount += 1
            await gate.wait()
            throw HealthDashboardTestError.failed
        }

        let first = Task { await store.load() }
        while loadCount == 0 {
            await Task.yield()
        }

        let second = Task {
            await store.load()
            await completion.set()
        }
        await Task.yield()
        await Task.yield()

        let completedBeforeRelease = await completion.value
        XCTAssertTrue(store.isLoading)
        XCTAssertFalse(completedBeforeRelease)
        XCTAssertEqual(loadCount, 1)

        await gate.open()
        await first.value
        await second.value

        let completedAfterRelease = await completion.value
        XCTAssertFalse(store.isLoading)
        XCTAssertTrue(completedAfterRelease)
        XCTAssertEqual(loadCount, 1)
        XCTAssertNotNil(store.errorMessage)
    }

    func testMissingAuthorizedSnapshotIsAnErrorRatherThanPermissionPrompt() {
        let state = DataSourceStatusStore.resolveHealthState(
            snapshot: nil,
            errorMessage: nil,
            reportedPermission: .authorized,
            cached: nil
        )

        XCTAssertEqual(state.phase, .error)
        XCTAssertEqual(state.permission, .authorized)
    }
}

private enum HealthDashboardTestError: LocalizedError {
    case failed

    var errorDescription: String? { "Expected test failure" }
}

private actor HealthDashboardTestFlag {
    private(set) var value = false

    func set() {
        value = true
    }
}

private actor HealthDashboardTestGate {
    private var isOpen = false
    private var continuations: [CheckedContinuation<Void, Never>] = []

    func wait() async {
        guard !isOpen else { return }
        await withCheckedContinuation { continuation in
            continuations.append(continuation)
        }
    }

    func open() {
        isOpen = true
        let pending = continuations
        continuations.removeAll()
        pending.forEach { $0.resume() }
    }
}

@MainActor
final class PromptTemplateAuthorizationTests: XCTestCase {
    private final class Persistence: PromptTemplatePersistence, @unchecked Sendable {
        var value: PromptTemplateSnapshot?
        var writes = 0
        init(_ value: PromptTemplateSnapshot?) { self.value = value }
        func load() throws -> PromptTemplateSnapshot? { value }
        func save(_ snapshot: PromptTemplateSnapshot) throws { writes += 1; value = snapshot }
    }

    func testLoadingUnownedTemplateDoesNotAdoptOrRewriteIt() throws {
        let original = PromptTemplate(name: "Unowned", content: "private", description: "")
        let persistence = Persistence(.init(customTemplates: [original], selectedTemplateID: original.id))
        let service = PromptTemplateService(logger: Logger(), persistence: persistence)
        XCTAssertEqual(persistence.writes, 0)
        XCTAssertNil(service.templates.first(where: { $0.id == original.id })?.accessOwnerID)
        XCTAssertThrowsError(try service.instructionTemplate(id: original.id))
        XCTAssertThrowsError(try service.getTemplate(for: "en"))
        XCTAssertEqual(persistence.value?.customTemplates, [original])
        XCTAssertEqual(persistence.value?.selectedTemplateID, original.id)
        XCTAssertEqual(persistence.writes, 0)
    }

    func testMissingExplicitSelectionNeverFallsBackToBuiltin() throws {
        let missing = UUID()
        let persistence = Persistence(.init(customTemplates: [], selectedTemplateID: missing))
        let service = PromptTemplateService(logger: Logger(), persistence: persistence)
        service.selectBuiltInLanguage(preferredLanguages: ["en"])
        XCTAssertThrowsError(try service.getCurrentTemplate()) { XCTAssertEqual($0 as? PromptTemplateReadError, .missing(missing)) }
        XCTAssertEqual(service.selectedTemplateID, missing)
        XCTAssertEqual(persistence.writes, 0)
    }

    func testBuiltinIdentityIsStableAndLanguageFilterDoesNotRemoveDefinitions() throws {
        let first = BuiltInPromptTemplates.templates()
        let second = BuiltInPromptTemplates.templates()
        XCTAssertEqual(first.count, 18)
        XCTAssertEqual(first.map(\.id), second.map(\.id))
        XCTAssertEqual(Set(first.map(\.id)).count, 18)
        let service = PromptTemplateService(logger: Logger())
        service.selectBuiltInLanguage(preferredLanguages: ["en"])
        XCTAssertEqual(Set(service.templates.map(\.id)), Set(first.map(\.id)))
        for definition in first { XCTAssertEqual(try service.instructionTemplate(id: definition.id).content, definition.content) }
    }

    func testStrictRecordRejectsUnknownAuthorityAndMissingRequiredDates() throws {
        let original = PromptTemplate(name: "Current", content: "body", description: "")
        var value = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(original)) as? [String: Any])
        value["host_id"] = "not-a-supported-template-field"
        XCTAssertThrowsError(try JSONDecoder().decode(PromptTemplate.self, from: JSONSerialization.data(withJSONObject: value)))
        value.removeValue(forKey: "host_id")
        value.removeValue(forKey: "createdAt")
        XCTAssertThrowsError(try JSONDecoder().decode(PromptTemplate.self, from: JSONSerialization.data(withJSONObject: value)))
    }

    func testNullAndAbsentAuthorityFieldsRoundTripWithoutNormalization() throws {
        let original = PromptTemplate(name: "Current", content: "body", description: "")
        var record = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(original)) as? [String: Any])
        record["accessOwnerID"] = NSNull()
        record["accessGrants"] = NSNull()
        let root: [String: Any] = ["customTemplates": [record], "selectedTemplateID": NSNull()]
        let decoded = try JSONDecoder().decode(PromptTemplateSnapshot.self, from: JSONSerialization.data(withJSONObject: root))
        let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(decoded)) as? NSDictionary)
        XCTAssertEqual(encoded, root as NSDictionary)
        var unknown = root
        unknown["hostId"] = "wrong"
        XCTAssertThrowsError(try JSONDecoder().decode(PromptTemplateSnapshot.self, from: JSONSerialization.data(withJSONObject: unknown)))
    }

    func testExpertGuidanceUsesActualParameterProducerAndKeepsExactBytes() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.responseStyle = .casual
        parameters.customSystemPrompt = "  Keep the precise source wording.\n第二行  "
        let captured = IOSExpertInstructionProducer.definitions(parameters: parameters)
        XCTAssertEqual(captured.map(\.component_id), ["ios:user-response-style", "ios:user-custom-instructions"])
        XCTAssertEqual(captured[0].content, PromptTemplates.agentResponseStyle(parameters.responseStyle))
        XCTAssertEqual(captured[1].content, "Keep the precise source wording.\n第二行")
        parameters.customSystemPrompt = "later configuration"
        XCTAssertEqual(captured[1].content, "Keep the precise source wording.\n第二行", "A captured value does not re-read later settings")
        XCTAssertNotEqual(captured[1].content_sha256, IOSExpertInstructionProducer.definitions(parameters: parameters)[1].content_sha256)
    }

    func testPreviewPresentationFeedsSameNativeDecoderWithoutChangingModelGeneration() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.25
        let updated = try ConversationParameterSnapshot.applyingPresentation(.object([
            "responseStyle": .string(PromptResponseStyle.casual.rawValue), "customSystemPrompt": .string("exact custom instruction"),
            "showThinking": .bool(true), "streamOutput": .bool(true),
        ]), to: parameters)
        XCTAssertEqual(updated.temperature, 0.25)
        XCTAssertEqual(updated.maxOutputTokens, parameters.maxOutputTokens)
        XCTAssertEqual(IOSExpertInstructionProducer.definitions(parameters: updated)[1].content, "exact custom instruction")
        XCTAssertThrowsError(try ConversationParameterSnapshot.applyingPresentation(.object(["customSystemPrompt": .bool(false)]), to: parameters))
    }

    func testPrivateHostDefiniteRejectionRetiresOnlyItsOwnPendingIntent() throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("Application ABI is unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("fixture.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let input = ApplePrepareEditedResend(operation_id: UUID().uuidString.lowercased(),
            source_conversation_id: "missing-source", source_message_id: "missing-message", expected_source_revision: 1,
            expected_source_message_revision: 1, edited_text: "new text", target_conversation_id: UUID().uuidString,
            title: "New", created_at_ms: 1)
        // This is a real Core rejection against an isolated Store. No model,
        // transport mock or fabricated success receipt participates.
        for _ in 0..<2 {
            XCTAssertThrowsError(try runtime.expertHostOperation(.prepareResend(input: input, authority: .local())) as ApplePreparedExpertResend<AgentCoreJSONValue>) {
                guard case IOSApplicationRuntimeError.rejected = $0 else { return XCTFail("Expected a definite Core rejection: \($0)") }
            }
            let journal = ApplePendingCommandJournal(databaseURL: database, namespace: .expertHost,
                validateOwners: AppleExpertHostIntentScope.validateJournalOwners)
            let transaction = try journal.beginTransaction()
            defer { transaction.close() }
            XCTAssertTrue(try transaction.load().isEmpty)
        }
    }
}

final class PromptTemplateCanonicalEndToEndTests: XCTestCase {
    func testPromptTemplatesColdReopenIgnoreLegacyDefaults() throws {
        let database = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-prompts-\(UUID().uuidString).sqlite")
        defer { try? FileManager.default.removeItem(at: database) }

        let legacy = PromptTemplate(name: "Legacy", content: "legacy", description: "legacy")
        UserDefaults.standard.set(try JSONEncoder().encode([legacy]), forKey: "promptTemplates")
        UserDefaults.standard.set(legacy.id.uuidString, forKey: "selectedPromptTemplateID")
        defer {
            UserDefaults.standard.removeObject(forKey: "promptTemplates")
            UserDefaults.standard.removeObject(forKey: "selectedPromptTemplateID")
        }

        let canonical = PromptTemplate(name: "Canonical", content: "canonical", description: "canonical")
        do {
            let store = try IOSApplicationRuntime(databaseURL: database)
            let snapshot = PromptTemplateSnapshot(
                customTemplates: [canonical],
                selectedTemplateID: canonical.id
            )
            let data = try JSONEncoder().encode(snapshot)
            _ = try store.putProductSetting(
                key: "prompt.templates",
                valueJSON: String(decoding: data, as: UTF8.self),
                updatedAtMs: 1
            )
        }

        do {
            let reopened = try IOSApplicationRuntime(databaseURL: database)
            let setting = try XCTUnwrap(reopened.productSetting(key: "prompt.templates"))
            let restored = try JSONDecoder().decode(
                PromptTemplateSnapshot.self,
                from: try XCTUnwrap(setting.valueJSON.data(using: .utf8))
            )
            XCTAssertEqual(restored.customTemplates.map(\.id), [canonical.id])
            XCTAssertFalse(restored.customTemplates.contains(where: { $0.id == legacy.id }))
            XCTAssertEqual(restored.selectedTemplateID, canonical.id)
        }
    }
}

final class ScheduledURLActionAgentRequestTests: XCTestCase {
    func testConfiguredPromptAndFetchedDataRemainSeparateTypedInputs() throws {
        let observedAt = Date(timeIntervalSince1970: 1_700_000_000)
        let request = try ScheduledURLActionAgentRequest.make(
            actionId: "action-1",
            sourceName: "Status Feed",
            sourceURL: "https://example.com/feed.json",
            userPrompt: "Report only meaningful changes.",
            sourceDescription: "Fallback description",
            dedupeKey: "scheduled:1",
            observedAt: observedAt,
            dateRange: .init(start: "2023-11-14T00:00:00Z", end: "2023-11-15T00:00:00Z"),
            candidates: [.init(
                type: "url",
                title: "Release 2.0",
                name: nil,
                date: nil,
                start: nil,
                due: nil,
                relation: nil,
                notesTag: nil,
                durationMin: nil
            )]
        )

        XCTAssertEqual(request.prompt, "Report only meaningful changes.")
        XCTAssertEqual(request.referenceComponent.kind, .document)
        XCTAssertEqual(request.referenceComponent.id, "action-1")
        XCTAssertEqual(request.referenceComponent.source?.authorization, "available")
        XCTAssertEqual(request.referenceComponent.source?.uri, "https://example.com/feed.json")
        XCTAssertEqual(request.referenceComponent.source?.observedAt, ISO8601DateFormatter().string(from: observedAt))
        guard case .string(let content) = request.referenceComponent.content else {
            return XCTFail("Expected JSON reference content")
        }
        let decoded = try JSONDecoder().decode(ModelInput.self, from: Data(content.utf8))
        XCTAssertEqual(decoded.candidates.first?.title, "Release 2.0")
        XCTAssertFalse(content.contains("Report only meaningful changes."))
    }
}

final class MemoryWorkerWakeSchedulingPolicyTests: XCTestCase {
    func testMemoryWorkerDeadlinePreemptsDailyAndScheduledWork() {
        let now = Date(timeIntervalSince1970: 1_000)
        let daily = now.addingTimeInterval(12 * 60 * 60)
        let scheduled = now.addingTimeInterval(4 * 60 * 60)
        let memory = now.addingTimeInterval(90)

        XCTAssertEqual(
            MemoryWorkerWakeSchedulingPolicy.nextRefreshDate(
                now: now,
                daily: daily,
                scheduledAction: scheduled,
                memoryWorker: memory
            ),
            memory
        )
        XCTAssertEqual(
            MemoryWorkerWakeSchedulingPolicy.nextProcessingDate(
                now: now,
                scheduledAction: scheduled,
                memoryWorker: memory
            ),
            memory
        )
    }

    func testOverdueMemoryWorkerDeadlineUsesMinimumSystemLead() {
        let now = Date(timeIntervalSince1970: 1_000)
        let expected = now.addingTimeInterval(MemoryWorkerWakeSchedulingPolicy.minimumLeadTime)

        XCTAssertEqual(
            MemoryWorkerWakeSchedulingPolicy.nextRefreshDate(
                now: now,
                daily: now.addingTimeInterval(10_000),
                scheduledAction: nil,
                memoryWorker: now.addingTimeInterval(-30)
            ),
            expected
        )
    }

    func testFailedMemoryWorkerAttemptsRetryWithBoundedBudget() {
        let now = Date(timeIntervalSince1970: 1_000)
        XCTAssertEqual(
            MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
                now: now,
                report: IOSMemoryWorkerRunReport(retryScheduled: 1),
                jobBudget: 2,
                retryAttemptCount: 1
            ),
            now.addingTimeInterval(MemoryWorkerWakeSchedulingPolicy.retryDelay)
        )
        XCTAssertEqual(
            MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
                now: now,
                report: IOSMemoryWorkerRunReport(retryScheduled: 1),
                jobBudget: 2,
                retryAttemptCount: MemoryWorkerWakeSchedulingPolicy.maximumRetryAttempts
            ),
            now.addingTimeInterval(MemoryWorkerWakeSchedulingPolicy.retryDelay)
        )
        XCTAssertNil(
            MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
                now: now,
                report: IOSMemoryWorkerRunReport(retryScheduled: 1),
                jobBudget: 2,
                retryAttemptCount: MemoryWorkerWakeSchedulingPolicy.maximumRetryAttempts + 1
            )
        )
    }

    func testFullJobBudgetSchedulesAnotherPass() {
        let now = Date(timeIntervalSince1970: 1_000)
        XCTAssertNotNil(
            MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
                now: now,
                report: IOSMemoryWorkerRunReport(attempted: 2, completed: 2),
                jobBudget: 2,
                retryAttemptCount: 0
            )
        )
        XCTAssertNil(
            MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
                now: now,
                report: IOSMemoryWorkerRunReport(attempted: 1, completed: 1),
                jobBudget: 2,
                retryAttemptCount: 0
            )
        )
        XCTAssertNil(
            MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
                now: now,
                report: IOSMemoryWorkerRunReport(),
                jobBudget: 2,
                retryAttemptCount: 0
            )
        )
    }
}

final class URLDataSourceBoundaryTests: XCTestCase {
    func testURLDataSourceClassifiesScheduledAutomation() {
        var urlAutomation = URLDataSource(
            url: "https://example.com/feed",
            format: .rss,
            name: "Daily Feed"
        )
        urlAutomation.scheduleFrequency = "daily"
        urlAutomation.scheduledTimeISO = "08:00"

        XCTAssertTrue(urlAutomation.isScheduledAutomation)
    }

    func testSystemDataSourcePolicyRejectsWriteWithoutRuntime() {
        let store = SystemDataSourceAccessPolicyStore(applicationRuntime: nil)
        XCTAssertThrowsError(try store.setPolicy(.blocked, for: .calendars))
    }

    func testSystemDataSourcePolicyPersistsAndRespectsConversationOverrides() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-source-policy-\(UUID().uuidString)", isDirectory: true)
        defer {
            try? FileManager.default.removeItem(at: root)
        }
        let store = SystemDataSourceAccessPolicyStore(
            applicationRuntime: try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        )

        XCTAssertEqual(store.policy(for: .health), .conversationOptIn)
        XCTAssertFalse(store.isEnabledForConversation(type: .health, explicitOverride: nil))

        try store.setPolicy(.defaultEnabled, for: .health)
        XCTAssertEqual(store.policy(for: .health), .defaultEnabled)
        XCTAssertTrue(store.isEnabledForConversation(type: .health, explicitOverride: nil))
        XCTAssertFalse(store.isEnabledForConversation(type: .health, explicitOverride: false))

        try store.setPolicy(.conversationOptIn, for: .health)
        XCTAssertTrue(store.isEnabledForConversation(type: .health, explicitOverride: true))

        try store.setPolicy(.blocked, for: .health)
        XCTAssertFalse(store.isEnabledForConversation(type: .health, explicitOverride: true))
        XCTAssertEqual(store.conversationOverrideKey(for: .health), "system:health")
    }
}

struct URLHistoryCanonicalEndToEndTests {
    @Test
    @MainActor
    func appendDeduplicatesAndSurvivesColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-url-history-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let entry = URLDataSource.HistoryEntry(
            timestamp: Date(timeIntervalSince1970: 1_750_000_000),
            dedupeKey: "article-42",
            title: "Canonical history",
            eventId: "event-42"
        )
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let entryData = try encoder.encode(entry)

        weak var firstRuntime: IOSApplicationRuntime?
        try autoreleasepool {
            let store = try IOSApplicationRuntime(databaseURL: database)
            firstRuntime = store
            let history = URLHistoryStore(applicationRuntime: store)
            #expect(history.append(sourceID: "source-1", entryData: entryData))
            #expect(history.append(sourceID: "source-1", entryData: entryData))
            #expect(history.load(sourceID: "source-1").count == 1)
        }

        // Release the MainActor-owned history wrapper on its owning executor,
        // and verify a cold reopen is not racing the first writer's teardown.
        #expect(firstRuntime == nil)

        let reopenedStore = try IOSApplicationRuntime(databaseURL: database)
        let reopened = URLHistoryStore(applicationRuntime: reopenedStore)
            .load(sourceID: "source-1")
        #expect(reopened.count == 1)
        #expect(reopened.first?.dedupeKey == entry.dedupeKey)
        #expect(reopened.first?.eventId == entry.eventId)
        #expect(reopened.first?.timestamp == entry.timestamp)
    }
}

final class UserTaskServiceTests: XCTestCase {
    func testUserTaskDefinitionSurvivesCanonicalColdReopenAndDeletesAsTombstone() throws {
        let databaseURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-user-task-\(UUID().uuidString).db")
        defer { try? FileManager.default.removeItem(at: databaseURL) }
        let createdAt = Date(timeIntervalSince1970: 1_778_000_000)
        let task = UserTask(
            id: UUID(),
            type: .reminder,
            title: "Review architecture",
            description: "Confirm the Application ownership boundary",
            targetDate: createdAt.addingTimeInterval(3_600),
            createdAt: createdAt,
            updatedAt: createdAt
        )

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: databaseURL)
            let storage = try LocalJSONStorage(applicationRuntime: runtime)
            try storage.saveUserTask(task)
            XCTAssertEqual(try storage.getUserTask(id: task.id), task)
        }

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: databaseURL)
            let storage = try LocalJSONStorage(applicationRuntime: runtime)
            XCTAssertEqual(try storage.getAllUserTasks(), [task])
            try storage.deleteUserTask(id: task.id)
            XCTAssertNil(try storage.getUserTask(id: task.id))
            XCTAssertTrue(try storage.getAllUserTasks().isEmpty)
        }
    }
}

@MainActor
final class OfficeDocumentTextExtractorTests: XCTestCase {
    func testAttachmentMessagePayloadPreservesSpecialFilenamesAndItemAlignment() throws {
        let first = ChatAttachmentDraft(
            name: "roadmap\n\"final\".png",
            typeLabel: "PNG",
            sizeBytes: 9_437_184,
            extractedContent: "",
            isImage: true,
            contentDigest: "missing-first"
        )
        let second = ChatAttachmentDraft(
            name: "预算 2026.xlsx",
            typeLabel: "XLSX",
            sizeBytes: 81_920,
            extractedContent: "sheet",
            contentDigest: "missing-second"
        )

        let encoded = try XCTUnwrap(ChatAttachmentMessagePayload.encode([first, second]))
        let decoded = ChatAttachmentMessagePayload.decode(from: [
            ChatAttachmentMessagePayload.metadataKey: encoded
        ])

        XCTAssertEqual(decoded.map(\.name), [first.name, second.name])
        XCTAssertEqual(decoded.map(\.sizeBytes), [Int64(first.sizeBytes), Int64(second.sizeBytes)])
        XCTAssertEqual(decoded.map(\.url), [nil, nil])
    }

    func testAttachmentMessagePayloadRejectsMalformedOrUnknownVersions() throws {
        XCTAssertTrue(ChatAttachmentMessagePayload.decode(from: [
            ChatAttachmentMessagePayload.metadataKey: "not-json"
        ]).isEmpty)

        let futurePayload = "{\"version\":2,\"items\":[{\"name\":\"a.png\",\"url\":null,\"sizeBytes\":1}]}"
        XCTAssertTrue(ChatAttachmentMessagePayload.decode(from: [
            ChatAttachmentMessagePayload.metadataKey: futurePayload
        ]).isEmpty)
    }

    func testExtractsBodyTableAndHeaderFromDOCXArchive() throws {
        let documentXML = """
        <?xml version="1.0" encoding="UTF-8"?>
        <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:body>
            <w:p><w:r><w:t>First paragraph</w:t></w:r></w:p>
            <w:tbl><w:tr>
              <w:tc><w:p><w:r><w:t>Cell A</w:t></w:r></w:p></w:tc>
              <w:tc><w:p><w:r><w:t>Cell B</w:t></w:r></w:p></w:tc>
            </w:tr></w:tbl>
          </w:body>
        </w:document>
        """
        let headerXML = """
        <?xml version="1.0" encoding="UTF-8"?>
        <w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:p><w:r><w:t>Report header</w:t></w:r></w:p>
        </w:hdr>
        """
        let archive = makeStoredZIP(entries: [
            ("word/document.xml", Data(documentXML.utf8)),
            ("word/header1.xml", Data(headerXML.utf8))
        ])

        let text = try OfficeDocumentTextExtractor.extractDOCX(from: archive)

        XCTAssertTrue(text.contains("First paragraph"))
        XCTAssertTrue(text.contains("Cell A"))
        XCTAssertTrue(text.contains("Cell B"))
        XCTAssertTrue(text.contains("Report header"))
    }

    func testRejectsArchiveWithoutDocumentBody() {
        let archive = makeStoredZIP(entries: [("word/styles.xml", Data("<styles/>".utf8))])
        XCTAssertThrowsError(try OfficeDocumentTextExtractor.extractDOCX(from: archive))
    }

    func testChatAttachmentPipelineImportsDOCXThroughTheUserUploadPath() async throws {
        let documentXML = """
        <?xml version="1.0" encoding="UTF-8"?>
        <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:body><w:p><w:r><w:t>Quarterly product roadmap</w:t></w:r></w:p></w:body>
        </w:document>
        """
        let archive = makeStoredZIP(entries: [("word/document.xml", Data(documentXML.utf8))])
        let url = try temporaryFile(named: "roadmap.docx", contents: archive)
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        let result = try await ChatAttachmentPipeline.importFiles(from: .success([url]), parserHost: nil)
        let attachment = try XCTUnwrap(result.attachments.first)

        XCTAssertEqual(result.failedCount, 0)
        XCTAssertEqual(result.attachments.count, 1)
        XCTAssertTrue(attachment.extractedContent.contains("Quarterly product roadmap"))
        XCTAssertNil(attachment.importWarning)
        let storedURL = try XCTUnwrap(attachment.localURL)
        defer { try? FileManager.default.removeItem(at: storedURL.deletingLastPathComponent()) }
        XCTAssertEqual(try Data(contentsOf: storedURL), archive)
    }

    func testChatAttachmentPipelineImportsSelectablePDFText() async throws {
        let renderer = UIGraphicsPDFRenderer(bounds: CGRect(x: 0, y: 0, width: 612, height: 792))
        let pdf = renderer.pdfData { context in
            context.beginPage()
            NSString(string: "Nola attachment pipeline PDF text").draw(
                at: CGPoint(x: 48, y: 64),
                withAttributes: [.font: UIFont.systemFont(ofSize: 18)]
            )
        }
        let url = try temporaryFile(named: "report.pdf", contents: pdf)
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        let result = try await ChatAttachmentPipeline.importFiles(from: .success([url]), parserHost: nil)
        let attachment = try XCTUnwrap(result.attachments.first)

        XCTAssertEqual(result.failedCount, 0)
        XCTAssertTrue(attachment.extractedContent.contains("Nola attachment pipeline PDF text"))
        XCTAssertNil(attachment.importWarning)
        let storedURL = try XCTUnwrap(attachment.localURL)
        defer { try? FileManager.default.removeItem(at: storedURL.deletingLastPathComponent()) }
        XCTAssertEqual(try Data(contentsOf: storedURL), pdf)
    }

    func testChatAttachmentPipelineRejectsFilesAboveThe30MBLimitBeforeReading() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-attachment-limit-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let url = root.appendingPathComponent("oversized.bin")
        FileManager.default.createFile(atPath: url.path, contents: Data())
        let handle = try FileHandle(forWritingTo: url)
        try handle.truncate(atOffset: UInt64(30 * 1024 * 1024 + 1))
        try handle.close()

        let result = try await ChatAttachmentPipeline.importFiles(from: .success([url]), parserHost: nil)

        XCTAssertTrue(result.attachments.isEmpty)
        XCTAssertEqual(result.failedCount, 1)
    }

    func testChatAttachmentPipelineRejectsInvalidSmallImagePayload() async throws {
        let url = try temporaryFile(
            named: "declared-image.png",
            contents: Data("this is not image data".utf8)
        )
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        let result = try await ChatAttachmentPipeline.importFiles(
            from: .success([url]),
            parserHost: nil
        )

        XCTAssertTrue(result.attachments.isEmpty)
        XCTAssertEqual(result.failedCount, 1)
    }

    private func temporaryFile(named name: String, contents: Data) throws -> URL {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-attachment-test-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let url = root.appendingPathComponent(name)
        try contents.write(to: url, options: .atomic)
        return url
    }

    private func makeStoredZIP(entries: [(String, Data)]) -> Data {
        var archive = Data()
        var centralDirectory = Data()

        for (name, payload) in entries {
            let nameData = Data(name.utf8)
            let localOffset = UInt32(archive.count)

            archive.appendLE(UInt32(0x0403_4B50))
            archive.appendLE(UInt16(20))
            archive.appendLE(UInt16(0))
            archive.appendLE(UInt16(0))
            archive.appendLE(UInt16(0))
            archive.appendLE(UInt16(0))
            archive.appendLE(UInt32(0))
            archive.appendLE(UInt32(payload.count))
            archive.appendLE(UInt32(payload.count))
            archive.appendLE(UInt16(nameData.count))
            archive.appendLE(UInt16(0))
            archive.append(nameData)
            archive.append(payload)

            centralDirectory.appendLE(UInt32(0x0201_4B50))
            centralDirectory.appendLE(UInt16(20))
            centralDirectory.appendLE(UInt16(20))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt32(0))
            centralDirectory.appendLE(UInt32(payload.count))
            centralDirectory.appendLE(UInt32(payload.count))
            centralDirectory.appendLE(UInt16(nameData.count))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt32(0))
            centralDirectory.appendLE(localOffset)
            centralDirectory.append(nameData)
        }

        let centralOffset = UInt32(archive.count)
        archive.append(centralDirectory)
        archive.appendLE(UInt32(0x0605_4B50))
        archive.appendLE(UInt16(0))
        archive.appendLE(UInt16(0))
        archive.appendLE(UInt16(entries.count))
        archive.appendLE(UInt16(entries.count))
        archive.appendLE(UInt32(centralDirectory.count))
        archive.appendLE(centralOffset)
        archive.appendLE(UInt16(0))
        return archive
    }
}

private extension Data {
    mutating func appendLE(_ value: UInt16) {
        append(UInt8(value & 0x00FF))
        append(UInt8((value >> 8) & 0x00FF))
    }

    mutating func appendLE(_ value: UInt32) {
        append(UInt8(value & 0x0000_00FF))
        append(UInt8((value >> 8) & 0x0000_00FF))
        append(UInt8((value >> 16) & 0x0000_00FF))
        append(UInt8((value >> 24) & 0x0000_00FF))
    }
}

final class IOSDataSourceHealthHybridAndMaintenanceSuiteTests: XCTestCase {
    func testDataSourceBatchReadAndLogFileStorePath() async throws {
        let values = try await DataSourceBatchRead.collect(reads: [{ @Sendable in [1, 2] }, { @Sendable in [3] }])
        XCTAssertEqual(values.sorted(), [1, 2, 3])

        enum Failure: Error { case offline }
        do {
            let _: [Int] = try await DataSourceBatchRead.collect(reads: [{ @Sendable in [1] }, { @Sendable in throw Failure.offline }])
            XCTFail("A partial result cannot be displayed as a complete event count")
        } catch { XCTAssertTrue(error is Failure) }

        do {
            let _: [Int] = try await DataSourceBatchRead.collect(reads: [{ @Sendable in throw CancellationError() }])
            XCTFail("Cancellation must reach the view without changing its last successful count")
        } catch { XCTAssertTrue(error is CancellationError) }

        let logURL = LogFileStore.shared.logFileURL()
        XCTAssertEqual(logURL.lastPathComponent, "nola.log")
        XCTAssertEqual(logURL.deletingLastPathComponent().lastPathComponent, "Logs")
        XCTAssertTrue(logURL.path.contains("Nola-Test-Runtime-")
            || logURL.deletingLastPathComponent().deletingLastPathComponent().lastPathComponent == "Nola")
    }

    @MainActor
    func testHealthStateResolutionHybridCapabilitiesAndSoftwareUpdate() async throws {
        let generatedAt = Date()
        let populated = HealthDashboardSnapshot(
            generatedAt: generatedAt, activityDate: generatedAt, todaySteps: 8_432,
            todayDistanceKilometers: 6.2, todayActiveEnergyKilocalories: 418, todayExerciseMinutes: 36,
            latestHeartRate: 72, latestHeartRateDate: generatedAt, restingHeartRate: 58,
            sleepHours: 7.4, sleepEndDate: generatedAt, latestWorkout: nil,
            dailySteps: [HealthDayValue(date: generatedAt, steps: 8_432)]
        )
        let readyState = DataSourceStatusStore.resolveHealthState(snapshot: populated, errorMessage: nil, reportedPermission: .notDetermined, cached: nil)
        XCTAssertEqual(readyState.phase, .ready)
        XCTAssertEqual(readyState.permission, .authorized)
        XCTAssertEqual(readyState.itemCount, populated.availableMetricCount)
        XCTAssertEqual(readyState.lastReadAt, generatedAt)

        let failedState = DataSourceStatusStore.resolveHealthState(snapshot: nil, errorMessage: "Health data is not readable", reportedPermission: .notDetermined, cached: nil)
        XCTAssertEqual(failedState.phase, .needsPermission)
        XCTAssertEqual(failedState.permission, .notDetermined)
        XCTAssertNil(failedState.lastReadAt)

        let emptySnapshot = HealthDashboardSnapshot(
            generatedAt: generatedAt, activityDate: generatedAt, todaySteps: 0,
            todayDistanceKilometers: 0, todayActiveEnergyKilocalories: 0, todayExerciseMinutes: 0,
            latestHeartRate: nil, latestHeartRateDate: nil, restingHeartRate: nil,
            sleepHours: nil, sleepEndDate: nil, latestWorkout: nil,
            dailySteps: [HealthDayValue(date: generatedAt, steps: 0)]
        )
        let emptyState = DataSourceStatusStore.resolveHealthState(snapshot: emptySnapshot, errorMessage: nil, reportedPermission: .notDetermined, cached: nil)
        XCTAssertFalse(emptySnapshot.hasReadableData)
        XCTAssertEqual(emptySnapshot.availableMetricCount, 0)
        XCTAssertEqual(emptyState.phase, .needsPermission)
        XCTAssertEqual(emptyState.permission, .notDetermined)
        XCTAssertNil(emptyState.itemCount); XCTAssertNil(emptyState.lastReadAt)

        // Hybrid capabilities canonical reopen, host rebind, and schema corruption rejection
        let database = FileManager.default.temporaryDirectory.appendingPathComponent("nola-hybrid-\(UUID().uuidString).sqlite")
        defer { try? FileManager.default.removeItem(at: database) }
        UserDefaults.standard.set(true, forKey: "chat.hybrid.device_tools_enabled")
        UserDefaults.standard.set(true, forKey: "chat.hybrid.files_enabled")
        defer {
            UserDefaults.standard.removeObject(forKey: "chat.hybrid.device_tools_enabled")
            UserDefaults.standard.removeObject(forKey: "chat.hybrid.files_enabled")
        }
        do {
            let store = try IOSApplicationRuntime(databaseURL: database)
            XCTAssertEqual(try IOSHybridCapabilitySettings.load(store: store), .init())
            try IOSHybridCapabilitySettings(
                boundHostURL: "https://host-a.example/v1", deviceToolsEnabled: true,
                filesEnabled: true, clipboardEnabled: true, locationEnabled: false, calendarEnabled: true
            ).save(store: store)
        }
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let persisted = try IOSHybridCapabilitySettings.load(store: reopened)
        XCTAssertTrue(persisted.deviceToolsEnabled && persisted.filesEnabled && persisted.clipboardEnabled && persisted.calendarEnabled)
        XCTAssertFalse(persisted.locationEnabled)
        try IOSHybridCapabilitySettings.bindHost("https://host-b.example/v1", store: reopened)
        let rebound = try IOSHybridCapabilitySettings.load(store: reopened)
        XCTAssertEqual(rebound.boundHostURL, "https://host-b.example/v1")
        XCTAssertFalse(rebound.deviceToolsEnabled || rebound.filesEnabled || rebound.clipboardEnabled || rebound.locationEnabled || rebound.calendarEnabled)
        for payload in [
            #"{"schemaVersion":0,"boundHostURL":"","deviceToolsEnabled":false,"filesEnabled":false,"clipboardEnabled":false,"locationEnabled":false,"calendarEnabled":false}"#,
            #"{"schemaVersion":1,"boundHostURL":"","deviceToolsEnabled":false,"filesEnabled":false,"clipboardEnabled":false,"locationEnabled":false,"calendarEnabled":false,"legacyFlag":true}"#
        ] {
            _ = try reopened.putProductSetting(key: IOSHybridCapabilitySettings.productSettingKey, valueJSON: payload, updatedAtMs: 1)
            XCTAssertThrowsError(try IOSHybridCapabilitySettings.load(store: reopened))
        }
        XCTAssertThrowsError(try reopened.putProductSetting(key: IOSHybridCapabilitySettings.productSettingKey, valueJSON: "not-json", updatedAtMs: 2))

        // Software update contract
        do {
            let _: SoftwareUpdateSnapshot = try await AppleSoftwareUpdateClient.call("install")
            XCTFail("The check/download ABI must never install or quit the Host")
        } catch {
            XCTAssertTrue(error is AppleSoftwareUpdateClient.UpdateFailure)
        }
        let offer = try JSONDecoder().decode(SoftwareUpdateSnapshot.self, from: Data(#"{"status":"available","current_version":"1.0.0","current_build":1,"channel":"internal","target":"ios","release":{"release_id":"signed-payload-digest","version":"1.0.1","build":2,"notes":"<literal release notes>","artifact":{"kind":"store","url":"https://apps.apple.com/app/id123"}}}"#.utf8))
        XCTAssertEqual(offer.release?.release_id, "signed-payload-digest")
        XCTAssertEqual(offer.release?.notes, "<literal release notes>")
        XCTAssertEqual(offer.release?.artifact.kind, "store")
    }
}
