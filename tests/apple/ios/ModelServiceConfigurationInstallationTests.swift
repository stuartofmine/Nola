import Foundation
import Testing
@testable import Nola

@MainActor
struct ModelServiceConfigurationInstallationTests {
    private enum FixtureTransportReached: Error { case reached }

    private final class SummaryProbe: LLMClient {
        var calls = 0
        func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
            calls += 1
            throw FixtureTransportReached.reached
        }
        func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
            AsyncThrowingStream { $0.finish(throwing: FixtureTransportReached.reached) }
        }
        func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
            throw FixtureTransportReached.reached
        }
    }

    @Test
    func summaryFacadeDoesNotRetryFailedPhysicalRequest() async throws {
        let provider = SummaryProbe()
        let service = ModelService(provider: provider)
        service.installTestingAuxiliaryLedger(try isolatedLedger())
        let input = ModelInput(dateRange: .init(start: "2026-10-15", end: "2026-10-15"), candidates: [], userPrefs: [:])
        await #expect(throws: FixtureTransportReached.self) {
            try await service.generateSummary(input: input, timeout: 1)
        }
        #expect(provider.calls == 1)
    }

    @Test
    func cancelledSummaryDoesNotDispatchProvider() async throws {
        let provider = SummaryProbe()
        let service = ModelService(provider: provider)
        let input = ModelInput(dateRange: .init(start: "2026-10-15", end: "2026-10-15"), candidates: [], userPrefs: [:])
        let task = Task { @MainActor in
            withUnsafeCurrentTask { $0?.cancel() }
            return try await service.generateSummary(input: input, timeout: 1)
        }
        await #expect(throws: CancellationError.self) { try await task.value }
        #expect(provider.calls == 0)
    }

    @Test
    func summaryInvocationPinsProviderRevisionAndHasOneAttemptOwner() async throws {
        let original = SummaryProbe()
        let replacement = SummaryProbe()
        let service = ModelService(provider: original)
        let ledger = try isolatedLedger()
        service.installTestingAuxiliaryLedger(ledger)
        let invocation = service.summaryInvocation(ownerID: "background-owner")
        service.installTestingProvider(replacement)
        let next = service.summaryInvocation(ownerID: "background-owner")
        #expect(next.configurationRevision > invocation.configurationRevision)
        #expect(next.operationID != invocation.operationID)
        let input = ModelInput(dateRange: .init(start: "2026-10-15", end: "2026-10-15"), candidates: [], userPrefs: [:])
        await #expect(throws: FixtureTransportReached.self) {
            try await invocation.generate(input: input, timeout: 1)
        }
        #expect(original.calls == 1)
        #expect(replacement.calls == 0)
        let receipt = try ledger.executeAuxiliaryInvocation([
            "type": .string("inspect"), "operation_id": .string(invocation.operationID.uuidString.lowercased())
        ])
        #expect(receipt.operation.attemptsStarted == 1)
        #expect(receipt.attempt?.state == "failed")
        #expect(receipt.attempt?.usage == .object(["coverage": .string("unknown"), "provider_raw": .null, "normalized": .null]))
    }

    @Test
    func fixtureTransportSurvivesProviderReconfigurationWithoutPinningModelSelection() async throws {
        let service = ModelService(provider: PlaceholderLocalProvider.shared)
        let config = WebAgentProvider.Config(
            baseURL: URL(string: "http://127.0.0.1:9")!, token: "fixture-token",
            model: "fixture-model", expectedHostId: "fixture-host", clientId: "fixture-client"
        )
        service.installTestingWebAgentTransport(baseURL: config.baseURL) { _ in
            throw FixtureTransportReached.reached
        }
        for _ in 0..<2 {
            service.useWebAgentProvider(config: config)
            let provider = try #require(service.currentProvider() as? WebAgentProvider)
            let identity = IOSRemoteSubmissionIdentity(
                source_host_id: "fixture-host", client_id: "fixture-client",
                conversation_id: "conversation", submission_id: "submission",
                user_message_id: "user", assistant_message_id: "assistant"
            )
            await #expect(throws: FixtureTransportReached.self) {
                try await provider.cancelHostGeneration(identity: identity, requestDigest: "fixture-digest")
            }
            service.usePlaceholderProvider()
            #expect(service.currentProvider() is PlaceholderLocalProvider)
        }
    }

    @Test
    func invalidInstallationRejectsBeforeReplacingTheCurrentProvider() throws {
        let original = OpenAIProvider(config: configuration())
        let service = ModelService(provider: original)
        var invalid = configuration()
        invalid.wireProtocol = .responses
        invalid.seed = 0
        #expect(throws: Error.self) { try service.useOpenAIProvider(config: invalid) }
        #expect(service.currentProvider() as? OpenAIProvider === original)
        try service.useOpenAIProvider(config: configuration())
        #expect(service.currentProvider() is OpenAIProvider)
        #expect(service.currentProvider() as? OpenAIProvider !== original)
    }

    private func configuration() -> OpenAIProvider.Config {
        .init(baseURL: URL(string: "https://fixture.invalid/v1")!, apiKey: "test-only",
            model: "fixture", maxCompletionTokens: 64)
    }

    private func isolatedLedger() throws -> IOSApplicationRuntime {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("nola-auxiliary-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("canonical.sqlite"))
    }
}
