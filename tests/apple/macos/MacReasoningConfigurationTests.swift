// SPDX-License-Identifier: Apache-2.0
import AppKit
import XCTest
@testable import Nola

final class MacReasoningConfigurationTests: XCTestCase {
    func testLaunchDescriptorKeepsOptionalSummaryOutOfBooleanCapabilities() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        for summary in [nil, false, true] as [Bool?] {
            let request = try MacLocalAgentRunRequest(
                conversationID: "summary-launch-fixture", prompt: "fixture", projectURL: root,
                attachmentURLs: [], modelID: "fixture",
                modelExecutionCapabilities: .init(reasoningSummarySupported: summary),
                appEnvironment: environment, applicationPort: port)
            let data = try await MacAgentHostRuntime().initialRequestData(for: request)
            let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            let descriptor = try XCTUnwrap(payload["mac_launch"] as? [String: Any])
            let capabilities = try XCTUnwrap(descriptor["model_capabilities"] as? [String: Bool])
            XCTAssertNil(capabilities["reasoning_summary_supported"])
            XCTAssertEqual(payload["reasoning_summary_supported"] as? Bool, summary)
        }
    }

    func testCanonicalRunKeepsCapturedCurrentPromptAndCoreOwnedHistory() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let host = MacAgentHostRuntime()
        for prompt in ["Explain blue sky. Begin SKY_REVIEW", "Now explain red sunsets. Begin SUNSET_REVIEW", "再解释一下", "再解释一下"] {
            let request = try MacLocalAgentRunRequest(conversationID: "turn-input-fixture",
                prompt: prompt, projectURL: root, attachmentURLs: [], modelID: "fixture", modelTestMode: true,
                appEnvironment: environment, applicationPort: port)
            let data = try await host.initialRequestData(for: request, localEndpoint: nil)
            let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(payload["prompt"] as? String, prompt)
            XCTAssertNil(payload["history"], "Canonical run must load history once from Core")
        }
    }

    func testSharedTurnPreparationPreservesRepeatedCommittedMessages() {
        let turn = ConversationTurnInput(currentUserText: "same", historySource: .prepared([
            .init(role: "system", content: "separate context"),
            .init(role: "user", content: "same"), .init(role: "assistant", content: "first answer")]))
        XCTAssertEqual(turn.input, "same")
        XCTAssertEqual(turn.history.map(\.role), ["user", "assistant"])
        XCTAssertEqual(turn.history.map(\.content), ["same", "first answer"])
        XCTAssertTrue(ConversationTurnInput(currentUserText: "first", historySource: .coreManaged).history.isEmpty)
    }

    func testNativeLaunchDoesNotDuplicateStdinPermissionPolicy() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let host = MacAgentHostRuntime()
        for mode in MacPermissionMode.allCases {
            let request = try MacLocalAgentRunRequest(
                conversationID: "permission-envelope-fixture", prompt: "fixture", projectURL: root,
                attachmentURLs: [], modelID: "fixture", modelTestMode: true,
                permissionMode: mode, appEnvironment: environment, applicationPort: port
            )
            let arguments = await host.arguments(for: request, localEndpoint: nil)
            XCTAssertFalse(arguments.contains("--yes"))
            XCTAssertFalse(arguments.contains("--network-policy"))
            XCTAssertTrue(arguments.contains("--host-request-stdin"))
            let data = try await host.initialRequestData(for: request, localEndpoint: nil)
            let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(payload["permission_profile"] as? String, mode.permissionProfile)
        }
    }

    func testConditionalSamplingUsesResolvedNoneEvenForDefaultValues() async throws {
        let payload = try await samplingPayload(effort: "none", capabilities: .init(
            reasoning: true, samplingParameters: true, samplingRequiresNoReasoning: true))
        XCTAssertEqual(payload["sampling_override"] as? Bool, true)
        XCTAssertEqual(payload["temperature"] as? Double, 0.7)
        XCTAssertEqual(payload["top_p"] as? Double, 0.9)
        XCTAssertEqual((payload["reasoning_selection"] as? [String: Any])?["value"] as? String, "none")
    }

    func testConditionalSamplingOmitsDefaultsForResolvedReasoning() async throws {
        let payload = try await samplingPayload(effort: "high", capabilities: .init(
            reasoning: true, samplingParameters: true, samplingRequiresNoReasoning: true))
        XCTAssertEqual(payload["sampling_override"] as? Bool, false)
    }

    func testExplicitNondefaultSamplingStillRejectsIncompatibleReasoning() async throws {
        do {
            _ = try await samplingPayload(effort: "high", capabilities: .init(
                reasoning: true, samplingParameters: true, samplingRequiresNoReasoning: true),
                explicit: true, temperature: 0)
            XCTFail("Explicit sampling cannot be silently omitted")
        } catch {
            XCTAssertEqual((error as NSError).domain, "NolaMac.LocalAgentRequest")
            XCTAssertEqual((error as NSError).code, 4)
        }
    }

    func testUnconditionalSamplingDeclarationSendsFrozenDefaults() async throws {
        let payload = try await samplingPayload(effort: "high", capabilities: .init(
            reasoning: true, samplingParameters: true))
        XCTAssertEqual(payload["sampling_override"] as? Bool, true)
    }

    func testUnknownSamplingCapabilityIsNotPromoted() async throws {
        let payload = try await samplingPayload(effort: nil, capabilities: nil)
        XCTAssertEqual(payload["sampling_override"] as? Bool, false)
    }

    private func samplingPayload(
        effort: String?, capabilities: MacModelExecutionCapabilities?,
        explicit: Bool = false, temperature: Double = 0.7
    ) async throws -> [String: Any] {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = temperature
        parameters.topP = 0.9
        parameters.maxOutputTokens = 4096
        let declaration = effort.map {
            MacReasoningConfiguration(control: .effort, responses: true, efforts: [$0])
        } ?? MacReasoningConfiguration()
        let request = try MacLocalAgentRunRequest(
            conversationID: "sampling-fixture", prompt: "fixture", projectURL: root,
            attachmentURLs: [], modelID: "fixture", modelTestMode: true,
            reasoningConfiguration: declaration, reasoningTier: "concise",
            modelParameters: parameters, modelExecutionCapabilities: capabilities,
            wireProtocol: .responses, samplingOverride: explicit,
            appEnvironment: environment, applicationPort: port
        )
        let data = try await MacAgentHostRuntime().initialRequestData(for: request)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    func testLocalMLXAlwaysSendsFrozenSamplingEvenWhenValuesEqualUIDefaults() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.7
        parameters.topP = 0.9
        parameters.maxOutputTokens = 4096
        let modelID = MacLocalModelProfile.qwen35_08B.id
        let request = try MacLocalAgentRunRequest(
            conversationID: "local-sampling", prompt: "fixture", projectURL: root,
            attachmentURLs: [], modelID: modelID, modelTestMode: true,
            modelParameters: parameters, wireProtocol: .chatCompletions,
            samplingOverride: false, appEnvironment: environment, applicationPort: port
        )
        let endpoint = MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:12345", apiKey: "fixture-token", modelID: modelID, contextWindowTokens: 32768, profileRevision: 1)
        let host = MacAgentHostRuntime()
        let arguments = await host.arguments(for: request, localEndpoint: endpoint)
        XCTAssertFalse(arguments.contains("--api-key"))
        XCTAssertFalse(arguments.contains(endpoint.apiKey))
        let data = try await host.initialRequestData(for: request, localEndpoint: endpoint)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(payload["api_key"] as? String, endpoint.apiKey)
        XCTAssertEqual(payload["sampling_override"] as? Bool, true)
        XCTAssertEqual(payload["temperature"] as? Double, 0.7)
        XCTAssertEqual(payload["top_p"] as? Double, 0.9)
    }

    func testApplicationReasoningResolutionUsesEffectiveProtocolAndSharedTierMapping() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("reasoning.db"))
        // Stale UI metadata must not choose the transport or weaken admission.
        let declaration = MacReasoningConfiguration(control: .effort, responses: false, efforts: ["high", "low"])
        let value = try port.resolveReasoning(
            declaration: declaration, wireProtocol: .responses, outputLimit: 4096,
            supportsReasoning: true, requested: .null, tier: "balanced"
        )
        XCTAssertEqual(value, .object(["kind": .string("effort"), "value": .string("low")]))
        XCTAssertEqual(try port.resolveReasoning(
            declaration: declaration, wireProtocol: .anthropic, outputLimit: 4096,
            supportsReasoning: true, requested: .null, tier: "balanced"
        ), .object(["kind": .string("effort"), "value": .string("low")]))
        XCTAssertThrowsError(try port.resolveReasoning(
            declaration: declaration, wireProtocol: .responses, outputLimit: 4096,
            supportsReasoning: false, requested: .null, tier: "balanced"
        ))
    }

    func testApplicationReasoningBudgetChecksOnlySelectedTierAgainstEffectiveOutput() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("reasoning.db"))
        let declaration = MacReasoningConfiguration(control: .budget)
        XCTAssertEqual(try port.resolveReasoning(
            declaration: declaration, wireProtocol: .anthropic, outputLimit: 4096,
            supportsReasoning: true, requested: .null, tier: "concise"
        ), .object(["kind": .string("budget"), "tokens": .number(1024)]))
        XCTAssertThrowsError(try port.resolveReasoning(
            declaration: declaration, wireProtocol: .anthropic, outputLimit: 4096,
            supportsReasoning: true, requested: .null, tier: "detailed"
        ))
        XCTAssertEqual(try port.resolveReasoning(
            declaration: nil, wireProtocol: .responses, outputLimit: 4096,
            supportsReasoning: false, requested: .null, tier: "detailed"
        ), .null)
    }

    func testNativeHostPayloadUsesApplicationResolutionAndFrozenContext() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.maxOutputTokens = 4096
        parameters.maxContextLength = 123456
        let request = try MacLocalAgentRunRequest(
            conversationID: "reasoning-fixture", prompt: "fixture", projectURL: root,
            attachmentURLs: [], modelID: "unknown-provider-model", modelTestMode: true,
            reasoningConfiguration: .init(control: .budget), reasoningPreference: "automatic",
            reasoningTier: "concise", modelParameters: parameters,
            modelExecutionCapabilities: .init(reasoning: true), wireProtocol: .anthropic,
            appEnvironment: environment, applicationPort: port
        )
        let data = try await MacAgentHostRuntime().initialRequestData(for: request)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let selection = try XCTUnwrap(payload["reasoning_selection"] as? [String: Any])
        XCTAssertEqual(selection["kind"] as? String, "budget")
        XCTAssertEqual(selection["tokens"] as? Int, 1024)
        XCTAssertEqual(payload["max_output_tokens"] as? Int, 4096)
        XCTAssertEqual(payload["context_window_tokens"] as? Int, 123456)
    }

    func testNativeHostDoesNotFallBackWhenApplicationPortIsMissing() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let request = try MacLocalAgentRunRequest(
            conversationID: "missing-port", prompt: "fixture", projectURL: root,
            attachmentURLs: [], modelTestMode: true,
            appEnvironment: MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root),
            applicationPort: nil
        )
        do {
            _ = try await MacAgentHostRuntime().initialRequestData(for: request)
            XCTFail("Missing Application must not silently use local reasoning logic")
        } catch {
            XCTAssertEqual(error as? MacApplicationPortError, .serviceUnavailable)
        }
    }

    func testNativeHostTierDoesNotSubmitTheOldLocalSelectionAsASecondOverride() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let request = try MacLocalAgentRunRequest(
            conversationID: "tier-precedence", prompt: "fixture", projectURL: root,
            attachmentURLs: [], modelID: "fixture", modelTestMode: true,
            reasoningConfiguration: .init(control: .effort, responses: true, efforts: ["low", "medium", "high"]),
            reasoningPreference: "low", reasoningTier: "balanced",
            modelExecutionCapabilities: .init(reasoning: true), wireProtocol: .responses,
            appEnvironment: environment, applicationPort: port
        )
        let data = try await MacAgentHostRuntime().initialRequestData(for: request)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let selection = try XCTUnwrap(payload["reasoning_selection"] as? [String: Any])
        XCTAssertEqual(selection["value"] as? String, "medium")
    }

    func testNativeHostPreservesAllFiveComposerEfforts() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let efforts = ["none", "low", "medium", "high", "xhigh"]
        for effort in efforts {
            let request = try MacLocalAgentRunRequest(
                conversationID: "exact-effort", prompt: "fixture", projectURL: root,
                attachmentURLs: [], modelID: "fixture", modelTestMode: true,
                reasoningConfiguration: .init(control: .effort, responses: true, efforts: efforts),
                reasoningPreference: effort,
                modelExecutionCapabilities: .init(reasoning: true), wireProtocol: .responses,
                appEnvironment: environment, applicationPort: port)
            let data = try await MacAgentHostRuntime().initialRequestData(for: request)
            let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            let selection = try XCTUnwrap(payload["reasoning_selection"] as? [String: Any])
            XCTAssertEqual(selection["value"] as? String, effort)
        }
    }

    func testBudgetEditorRetainsInvalidRawInputAndRejectsItsConfiguration() {
        for text in ["", "-", "1.5", "NaN", "123x", "9999999999999999999999999999"] {
            var draft = MacReasoningBudgetDraft()
            var configuration = MacReasoningConfiguration(control: .budget)
            configuration.budgets[0] = draft.update(text, at: 0)
            XCTAssertEqual(draft.text(at: 0, value: configuration.budgets[0]), text)
            XCTAssertFalse(configuration.isValid(for: .anthropic), "Invalid edit must not save the old 1024 budget: \(text)")
            configuration.budgets[0] = draft.update("2048", at: 0)
            XCTAssertTrue(configuration.isValid(for: .anthropic))
        }
    }

    func testBudgetEditorDisplaysReloadedValueInsteadOfStaleDraft() {
        var draft = MacReasoningBudgetDraft()
        XCTAssertEqual(draft.update("-", at: 0), -1)
        XCTAssertEqual(draft.text(at: 0, value: 2048), "2048")
        XCTAssertEqual(draft.update("002048", at: 0), 2048)
        XCTAssertEqual(draft.text(at: 0, value: 2048), "002048")
        XCTAssertEqual(draft.text(at: 1, value: 4096), "4096")
    }

    func testAutomaticDoesNotMakeInvalidBudgetDraftSaveable() {
        var draft = MacReasoningBudgetDraft()
        var configuration = MacReasoningConfiguration(control: .budget)
        configuration.budgets[0] = draft.update("-", at: 0)
        configuration.control = .automatic
        for wireProtocol in MacModelWireProtocol.allCases {
            XCTAssertFalse(configuration.isValid(for: wireProtocol))
            XCTAssertFalse(configuration.isValid(for: wireProtocol, preferenceOverride: "automatic"))
        }
        configuration.budgets[0] = draft.update("2048", at: 0)
        XCTAssertTrue(configuration.isValid(for: .anthropic))
    }

    func testInactiveReasoningFieldsStillObeySharedSchema() {
        for control in MacReasoningConfiguration.Control.allCases {
            var configuration = MacReasoningConfiguration(control: control, efforts: ["low", "medium", "high"])
            let wireProtocol: MacModelWireProtocol = control == .budget ? .anthropic : .responses
            XCTAssertTrue(configuration.isValid(for: wireProtocol))
            configuration.efforts.append("unknown-effort")
            XCTAssertFalse(configuration.isValid(for: wireProtocol))
            configuration.efforts.removeLast()
            configuration.budgets[0] = -1
            XCTAssertFalse(configuration.isValid(for: wireProtocol))
        }
        XCTAssertTrue(MacReasoningConfiguration(control: .automatic, budgets: [0]).isValid(for: .responses),
                      "Nonnegative inactive budgets are schema-valid; three-tier constraints apply only to budget control")
    }

    func testUnsupportedDerivedEffortIsRejected() {
        let configuration = MacReasoningConfiguration(control: .effort, responses: true, efforts: ["low", "high"])
        XCTAssertFalse(configuration.isValid(for: .responses, preferenceOverride: "xhigh"))
        XCTAssertTrue(configuration.isValid(for: .chatCompletions, preferenceOverride: "high"))
    }

    func testDerivedPreferenceIsValidatedAgainstCurrentModelCapability() {
        let configuration = MacReasoningConfiguration(control: .effort, responses: true, efforts: ["low", "high"])
        XCTAssertTrue(configuration.isValid(for: .responses, preferenceOverride: nil))
        XCTAssertTrue(configuration.isValid(for: .responses, preferenceOverride: "low"))
        XCTAssertTrue(configuration.isValid(for: .responses, preferenceOverride: "automatic"))
        XCTAssertFalse(configuration.isValid(for: .responses, preferenceOverride: "medium"))
    }

    func testModelCapabilityRoundTripsWithoutASecondDefaultLevel() throws {
        let configuration = MacReasoningConfiguration(control: .effort, responses: true, efforts: ["low", "high"])
        XCTAssertTrue(configuration.isValid(for: .responses))
        let restored = try JSONDecoder().decode(MacReasoningConfiguration.self, from: JSONEncoder().encode(configuration))
        XCTAssertEqual(restored, configuration)
        let encoded = try XCTUnwrap(String(data: JSONEncoder().encode(configuration), encoding: .utf8))
        XCTAssertFalse(encoded.contains("defaultPreference"))
    }

    func testConversationModelParametersStayWithTheirModelAcrossPersistence() throws {
        let first = MacConversationModelOverride(temperature: 0.25)
        let second = MacConversationModelOverride(maxOutputTokens: 8192)
        let overrides = ["configured:first": first, "configured:second": second]
        let restored = try JSONDecoder().decode([String: MacConversationModelOverride].self, from: JSONEncoder().encode(overrides))
        XCTAssertEqual(restored["configured:first"]?.temperature, 0.25)
        XCTAssertEqual(restored["configured:second"]?.maxOutputTokens, 8192)
        let encoded = try XCTUnwrap(String(data: JSONEncoder().encode(overrides), encoding: .utf8))
        XCTAssertFalse(encoded.contains("reasoning"), "Sampling-only overrides do not add a reasoning preference")
    }
    func testProtocolSwitchValidatesInheritedContractAgainstEffectiveProtocol() {
        let budget = MacReasoningConfiguration(control: .budget)
        XCTAssertTrue(budget.isValid(for: .anthropic))
        XCTAssertFalse(budget.isValid(for: .responses))
        let effort = MacReasoningConfiguration(control: .effort, efforts: ["high"])
        XCTAssertTrue(effort.isValid(for: .responses))
        XCTAssertTrue(effort.isValid(for: .anthropic))
        XCTAssertTrue(effort.isValid(for: .chatCompletions), "Exact effort controls preserve sparse declarations")
        var inheritedBudget = budget
        inheritedBudget.outputLimit = 4096
        XCTAssertFalse(inheritedBudget.isValid(for: .anthropic), "Session output limits must validate inherited budgets too")
    }
    func testThinkingControlsAreProtocolSpecific() {
        XCTAssertEqual(MacReasoningConfiguration.availableControls(for: .responses), [.automatic, .effort])
        XCTAssertEqual(MacReasoningConfiguration.availableControls(for: .chatCompletions), [.automatic, .effort])
        XCTAssertEqual(MacReasoningConfiguration.availableControls(for: .anthropic), [.automatic, .effort, .budget])
        XCTAssertFalse(MacReasoningConfiguration(control: .budget).supports(.responses))
        XCTAssertTrue(MacReasoningConfiguration(control: .effort).supports(.anthropic))
    }
    func testReasoningMenuLabelsAreBundled() throws {
        let path = try XCTUnwrap(Bundle.main.path(forResource: "Reasoning", ofType: "strings", inDirectory: nil, forLocalization: "en"))
        let data = try Data(contentsOf: URL(fileURLWithPath: path))
        let labels = try XCTUnwrap(PropertyListSerialization.propertyList(from: data, format: nil) as? [String: String])
        for effort in ["automatic", "none", "minimal", "low", "medium", "high", "xhigh", "max"] {
            XCTAssertNotNil(labels["reasoning.effort." + effort])
        }
    }

    func testComposerPreservesAllDeclaredEfforts() throws {
        let efforts = ["none", "low", "medium", "high", "xhigh"]
        for responses in [false, true] {
            let configuration = MacReasoningConfiguration(control: .effort, responses: responses, efforts: efforts)
            XCTAssertEqual(configuration.options, efforts)
            for effort in efforts {
                XCTAssertEqual(configuration.selection(effort)?["value"] as? String, effort)
                let override = MacConversationModelOverride(reasoningPreference: effort)
                let decoded = try JSONDecoder().decode(MacConversationModelOverride.self, from: JSONEncoder().encode(override))
                XCTAssertEqual(decoded.applying(to: ModelParameters()).reasoningPreference, effort)
            }
            XCTAssertNil(configuration.selection("balanced"))
        }
        let state = MacComposerDisplayState(conversationID: "test", draft: "", attachments: [],
            isRunning: false, isVisiblyRunning: false, reasoningOptions: efforts, reasoningPreference: "xhigh")
        XCTAssertEqual(state.reasoningOptions, efforts)
        XCTAssertEqual(state.reasoningPreference, "xhigh")
    }

    func testEachResponseDepthHasUserVisibleOutputGuidance() {
        let instructions = MacResponseStyle.allCases.map(\.responseInstruction)
        XCTAssertEqual(Set(instructions).count, 3)
        XCTAssertTrue(instructions.allSatisfy { !$0.isEmpty })
        XCTAssertTrue(MacResponseStyle.concise.responseInstruction.contains("concisely"))
        XCTAssertTrue(MacResponseStyle.detailed.responseInstruction.contains("thorough"))
    }

    func testResponseDepthInstructionSurvivesCoreContextPreparation() {
        let instruction = MacResponseStyle.concise.responseInstruction
        let messages = MacContextCore.hostContextMessages(
            userPrompt: "Summarize the results",
            components: [MacCoreContextComponent(
                kind: "prompt_template",
                id: "macos:response-depth",
                content: instruction
            )],
            contextWindowTokens: 32_000
        )
        XCTAssertTrue(messages.contains { $0.content.contains(instruction) })
    }

    @MainActor
    func testModelCenterNavigationTargetsSelectedRemoteProfile() {
        let navigation = MacModelCenterNavigationState()
        XCTAssertTrue(navigation.showsLocalModels)
        navigation.showRemoteModel(profileID: "current-profile")
        XCTAssertTrue(navigation.showsRemoteModels)
        XCTAssertEqual(navigation.focusedRemoteProfileID, "current-profile")
        let firstRevision = navigation.focusRevision
        navigation.showRemoteModel(profileID: "current-profile")
        XCTAssertGreaterThan(navigation.focusRevision, firstRevision,
                             "Opening the same profile again must refocus its editor")
        navigation.showLocalModels()
        XCTAssertTrue(navigation.showsLocalModels)
        navigation.showSelectedModel(remoteProfileID: "next-profile")
        XCTAssertEqual(navigation.focusedRemoteProfileID, "next-profile")
        navigation.showSelectedModel(remoteProfileID: nil)
        XCTAssertTrue(navigation.showsLocalModels)
        XCTAssertNil(navigation.focusedRemoteProfileID)
    }

    @MainActor
    func testPermissionChipUsesTypedActionWithoutSubmittingDraft() {
        var selected: MacPermissionMode?
        let sink = MacComposerActionSink { action in
            guard case .selectPermissionMode(let conversationID, let mode) = action else {
                XCTFail("Permission selection must not submit or rewrite a draft")
                return false
            }
            XCTAssertEqual(conversationID, "conversation")
            selected = mode
            return true
        }
        XCTAssertTrue(sink.perform(.selectPermissionMode(conversationID: "conversation", .cautious)))
        XCTAssertEqual(selected, .cautious)
    }
    @MainActor
    func testModelReasoningContractSurvivesCanonicalReload() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        let reasoning = MacReasoningConfiguration(control: .effort, responses: true, efforts: ["low", "high"])
        try store.replaceEndpoints([MacRemoteEndpointConfiguration(
            id: "endpoint", name: "fixture", baseURL: "https://fixture.invalid/v1",
            models: [MacRemoteModelConfiguration(id: "model", modelName: "fixture",
                executionCapabilities: MacModelExecutionCapabilities(reasoning: true,
                    chatOutputTokenField: .maxCompletionTokens), reasoning: reasoning)],
            wireProtocol: .responses
        )])
        XCTAssertEqual(store.endpoints().first?.models.first?.reasoning, reasoning)
        XCTAssertEqual(store.load(model: "model")?.reasoning, reasoning)
        XCTAssertEqual(store.routePlan(preferredModel: "model")?.primary.reasoning, reasoning)
        XCTAssertEqual(store.routePlan(preferredModel: "model")?.primary.executionCapabilities.chatOutputTokenField,
            .maxCompletionTokens)
    }

    func testUnknownCapabilityDoesNotAdvertiseFakeControls() {
        XCTAssertTrue(MacReasoningConfiguration().options.isEmpty)
        XCTAssertNil(MacReasoningConfiguration().selection("high"))
    }

    @MainActor
    func testModelNameDoesNotInventSupportedReasoningEfforts() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("reasoning-metadata-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = MacAppleChatConfigurationStore(
            applicationPort: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        )
        try store.replaceEndpoints([MacRemoteEndpointConfiguration(
            id: "endpoint", name: "Gateway", baseURL: "https://example.invalid/v1",
            models: [MacRemoteModelConfiguration(id: "named", modelName: "gpt-5.5", executionCapabilities: .init())],
            wireProtocol: .responses
        )])
        XCTAssertNil(store.load(model: "named")?.reasoning)
        XCTAssertTrue(MacReasoningConfiguration().options.isEmpty)
    }

    func testResponsesUsesOnlyDeclaredEfforts() {
        let config = MacReasoningConfiguration(control: .effort, responses: true, efforts: ["low", "high", "xhigh"])
        XCTAssertEqual(config.options, ["low", "high", "xhigh"])
        XCTAssertNil(config.selection("medium"))
        XCTAssertNil(config.selection("automatic"))
        XCTAssertEqual(config.selection("xhigh")?["value"] as? String, "xhigh")
    }

    func testChatCompletionsPreservesExactEfforts() {
        let config = MacReasoningConfiguration(control: .effort, efforts: ["low", "medium", "high"])
        XCTAssertEqual(config.options, ["low", "medium", "high"])
        XCTAssertEqual(config.selection("medium")?["value"] as? String, "medium")
    }

    func testBudgetsAreExplicitAndBelowOutputLimit() {
        var config = MacReasoningConfiguration(control: .budget, budgets: [1024, 2048, 4096], outputLimit: 8192)
        XCTAssertEqual(config.selection("4096")?["tokens"] as? Int, 4096)
        config.outputLimit = 4096
        XCTAssertTrue(config.options.isEmpty)
        config.outputLimit = 8192
        config.responses = true
        XCTAssertTrue(config.options.isEmpty)
    }

    @MainActor
    func testNativePlaceholderDoesNotEnterDraftOrUndoHistory() {
        let view = MacAttachmentPasteTextView(frame: NSRect(x: 0, y: 0, width: 320, height: 30))
        view.placeholder = "给 Nola 发消息"
        view.font = .systemFont(ofSize: 14.5)
        view.textContainerInset = NSSize(width: 0, height: 5)
        view.textContainer?.lineFragmentPadding = 0
        XCTAssertEqual(view.string, "")
        XCTAssertEqual(view.textContainerOrigin.x, 0)
        XCTAssertEqual(view.textContainerOrigin.y, 5)
        XCTAssertEqual(view.selectedRange(), NSRange(location: 0, length: 0))
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            view.appearance = NSAppearance(named: name)
            view.layoutSubtreeIfNeeded()
            let bitmap = try! XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            let image = NSImage(size: view.bounds.size)
            image.addRepresentation(bitmap)
            let attachment = XCTAttachment(image: image)
            attachment.name = "composer-native-placeholder-\(name.rawValue)"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
    }
}
