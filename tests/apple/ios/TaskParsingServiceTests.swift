// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements TaskParsingServiceTests logic in the project module.
// - Interfaces:
//   - URLDataSourceBoundaryTests
// - APIs:
//   - none
//
import XCTest
@testable import Nola

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

        store.setPolicy(.defaultEnabled, for: .health)
        XCTAssertEqual(store.policy(for: .health), .defaultEnabled)
        XCTAssertTrue(store.isEnabledForConversation(type: .health, explicitOverride: nil))
        XCTAssertFalse(store.isEnabledForConversation(type: .health, explicitOverride: false))

        store.setPolicy(.conversationOptIn, for: .health)
        XCTAssertTrue(store.isEnabledForConversation(type: .health, explicitOverride: true))

        store.setPolicy(.blocked, for: .health)
        XCTAssertFalse(store.isEnabledForConversation(type: .health, explicitOverride: true))
        XCTAssertEqual(store.conversationOverrideKey(for: .health), "system:health")
    }
}

@MainActor
private final class StubTaskParsingAgentModelClient: AgentModelClient {
    let responses: [AgentCoreJSONValue]
    let submitToolCall: Bool
    private var responseIndex = 0

    init(response: String, submitToolCall: Bool = true) {
        self.responses = [try! JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(response.utf8))]
        self.submitToolCall = submitToolCall
    }

    init(responses: [String]) {
        self.responses = responses.map {
            try! JSONDecoder().decode(AgentCoreJSONValue.self, from: Data($0.utf8))
        }
        self.submitToolCall = true
    }

    func generateAgentTurn(request: AgentModelRequest) -> AsyncThrowingStream<AgentModelEvent, Error> {
        AsyncThrowingStream { continuation in
            let arguments = responses[min(responseIndex, responses.count - 1)]
            responseIndex += 1
            let calls = submitToolCall
                ? [AgentCoreToolCall(id: "task-parse", name: "submit_task_parse", arguments: arguments)]
                : []
            continuation.yield(.completed(.init(content: submitToolCall ? "" : responseText, toolCalls: calls)))
            continuation.finish()
        }
    }

    private var responseText: String {
        let arguments = responses[min(responseIndex, responses.count - 1)]
        return String(decoding: try! JSONEncoder().encode(arguments), as: UTF8.self)
    }
}

@MainActor
final class TaskParsingServiceDecisionTests: XCTestCase {
    func testTaskParsingPromptKeepsImmediateAssistantRequestsOutOfTaskFlow() {
        let prompt = PromptTemplates.taskParsingSystem()

        XCTAssertTrue(prompt.contains("Immediate requests directed at the assistant are NOT tasks"))
        XCTAssertTrue(prompt.contains("背诵赤壁赋"))
        XCTAssertTrue(prompt.contains("\"intent\": \"none\""))
        XCTAssertTrue(prompt.contains("submit_task_parse"))
        XCTAssertFalse(prompt.contains("###JSON_START###"))
    }

    func testParseInstructionReturnsNoActionWhenIntentIsNone() async {
        let response = """
        {
          "intent": "none",
          "type": null,
          "title": null,
          "description": null,
          "targetDate": null,
          "isRecurring": null,
          "recurrenceRule": null,
          "relatedPerson": null,
          "amount": null,
          "currency": null,
          "dueDate": null,
          "scheduledAction": null,
          "confidence": 0.2,
          "reason": "No actionable task detected."
        }
        """

        let service = TaskParsingService(agentModelClient: StubTaskParsingAgentModelClient(response: response))
        let result = await service.parseInstruction("你还记得我喜欢什么吗")

        XCTAssertFalse(result.success)
        XCTAssertNil(result.task)
        XCTAssertNil(result.scheduledAction)
        XCTAssertEqual(result.confidence, 0.2, accuracy: 0.0001)
    }

    func testParseInstructionReturnsNoActionWhenReasonSaysNotActionable() async {
        let response = """
        {
          "intent": "user_task",
          "type": "reminder",
          "title": "Check user preferences",
          "description": "User asked if preferences are remembered.",
          "targetDate": null,
          "isRecurring": false,
          "recurrenceRule": null,
          "relatedPerson": null,
          "amount": null,
          "currency": null,
          "dueDate": null,
          "scheduledAction": null,
          "confidence": 0.5,
          "reason": "No actionable task detected."
        }
        """

        let service = TaskParsingService(agentModelClient: StubTaskParsingAgentModelClient(response: response))
        let result = await service.parseInstruction("你还记得我喜欢什么吗")

        XCTAssertFalse(result.success)
        XCTAssertNil(result.task)
        XCTAssertNil(result.scheduledAction)
        XCTAssertEqual(result.confidence, 0.5, accuracy: 0.0001)
    }

    func testParseInstructionStillReturnsTaskWhenActionable() async {
        let response = """
        {
          "intent": "user_task",
          "type": "reminder",
          "title": "买牛奶",
          "description": null,
          "targetDate": "2099-01-01T09:00:00",
          "isRecurring": false,
          "recurrenceRule": null,
          "relatedPerson": null,
          "amount": null,
          "currency": null,
          "dueDate": null,
          "scheduledAction": null,
          "confidence": 0.93,
          "reason": null
        }
        """

        let service = TaskParsingService(agentModelClient: StubTaskParsingAgentModelClient(response: response))
        let result = await service.parseInstruction("明天九点提醒我买牛奶")

        XCTAssertTrue(result.success)
        XCTAssertNotNil(result.task)
        XCTAssertNil(result.scheduledAction)
        XCTAssertEqual(result.task?.title, "买牛奶")
        XCTAssertEqual(result.confidence, 0.93, accuracy: 0.0001)
    }

    func testParseInstructionAcceptsStructuredRecurrenceRule() async {
        let response = """
        {
          "intent": "user_task",
          "type": "reminder",
          "title": "站会",
          "description": null,
          "targetDate": "2099-01-01T09:00:00",
          "isRecurring": true,
          "recurrenceRule": { "frequency": "weekly", "interval": 1 },
          "relatedPerson": null,
          "amount": null,
          "currency": null,
          "dueDate": null,
          "scheduledAction": null,
          "confidence": 0.91,
          "reason": null
        }
        """

        let service = TaskParsingService(agentModelClient: StubTaskParsingAgentModelClient(response: response))
        let result = await service.parseInstruction("每周一九点提醒我开站会")

        XCTAssertTrue(result.success)
        XCTAssertEqual(result.task?.recurrenceRule?.frequency, .weekly)
        XCTAssertEqual(result.task?.recurrenceRule?.interval, 1)
    }

    func testParseInstructionAcceptsStructuredRecurrenceInterval() async {
        let response = """
        {
          "intent": "user_task",
          "type": "reminder",
          "title": "喝水",
          "description": null,
          "targetDate": "2099-01-01T09:00:00",
          "isRecurring": true,
          "recurrenceRule": { "frequency": "daily", "interval": 2 },
          "relatedPerson": null,
          "amount": null,
          "currency": null,
          "dueDate": null,
          "scheduledAction": null,
          "confidence": 0.91,
          "reason": null
        }
        """

        let service = TaskParsingService(agentModelClient: StubTaskParsingAgentModelClient(response: response))
        let result = await service.parseInstruction("每两天提醒我喝水")

        XCTAssertTrue(result.success)
        XCTAssertEqual(result.task?.recurrenceRule?.frequency, .daily)
        XCTAssertEqual(result.task?.recurrenceRule?.interval, 2)
    }

    func testParseInstructionParsesNaiveDueDateForDebtTasks() async {
        let response = """
        {
          "intent": "user_task",
          "type": "debt",
          "title": "还信用卡",
          "description": null,
          "targetDate": null,
          "isRecurring": false,
          "recurrenceRule": null,
          "relatedPerson": null,
          "amount": 1200,
          "currency": "CNY",
          "dueDate": "2099-02-03T18:30:00",
          "scheduledAction": null,
          "confidence": 0.9,
          "reason": null
        }
        """

        let service = TaskParsingService(agentModelClient: StubTaskParsingAgentModelClient(response: response))
        let result = await service.parseInstruction("2099 年 2 月 3 日 18:30 提醒我还信用卡")

        XCTAssertTrue(result.success)
        let dueDate = try? XCTUnwrap(result.task?.dueDate)
        let components = Calendar.current.dateComponents([.year, .month, .day, .hour, .minute], from: dueDate ?? Date())
        XCTAssertEqual(components.year, 2099)
        XCTAssertEqual(components.month, 2)
        XCTAssertEqual(components.day, 3)
        XCTAssertEqual(components.hour, 18)
        XCTAssertEqual(components.minute, 30)
    }

    func testParseInstructionRejectsPlainTextInsteadOfRequiredToolCall() async {
        let response = """
        {
          "intent": "none",
          "type": null,
          "title": null,
          "description": null,
          "targetDate": null,
          "isRecurring": null,
          "recurrenceRule": null,
          "relatedPerson": null,
          "amount": null,
          "currency": null,
          "dueDate": null,
          "scheduledAction": null,
          "confidence": 0.9,
          "reason": "plain text is not a trusted structured submission"
        }
        """
        let client = StubTaskParsingAgentModelClient(response: response, submitToolCall: false)
        let result = await TaskParsingService(agentModelClient: client)
            .parseInstruction("普通对话")

        XCTAssertFalse(result.success)
        XCTAssertNil(result.task)
        XCTAssertNil(result.scheduledAction)
    }

    func testParseInstructionRejectsToolArgumentsMissingRequiredFields() async {
        let response = """
        { "intent": "user_task", "title": "Incomplete" }
        """
        let result = await TaskParsingService(
            agentModelClient: StubTaskParsingAgentModelClient(response: response)
        ).parseInstruction("提醒我")

        XCTAssertFalse(result.success)
        XCTAssertNil(result.task)
    }

    func testParseInstructionRetriesSchemaFailureWithTheSameRequiredTool() async {
        let incomplete = """
        { "intent": "user_task", "title": "Incomplete" }
        """
        let corrected = """
        {
          "intent": "user_task",
          "type": "reminder",
          "title": "Drink water",
          "description": null,
          "targetDate": "2099-01-01T09:00:00",
          "isRecurring": false,
          "recurrenceRule": null,
          "relatedPerson": null,
          "amount": null,
          "currency": null,
          "dueDate": null,
          "scheduledAction": null,
          "confidence": 0.9,
          "reason": null
        }
        """
        let result = await TaskParsingService(
            agentModelClient: StubTaskParsingAgentModelClient(responses: [incomplete, corrected])
        ).parseInstruction("提醒我喝水")

        XCTAssertTrue(result.success)
        XCTAssertEqual(result.task?.title, "Drink water")
    }

    func testTaskParseResultConfirmationIdentityDoesNotReuseSameConfidenceTasks() {
        let taskA = UserTask(id: UUID(), title: "A")
        let taskB = UserTask(id: UUID(), title: "B")
        let resultA = TaskParseResult(success: true, task: taskA, scheduledAction: nil, confidence: 0.9, raw: "")
        let resultB = TaskParseResult(success: true, task: taskB, scheduledAction: nil, confidence: 0.9, raw: "")

        XCTAssertNotEqual(resultA.confirmationIdentity, resultB.confirmationIdentity)
    }
}
