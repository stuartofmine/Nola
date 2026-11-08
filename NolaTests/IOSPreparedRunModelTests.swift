import Foundation
import Testing
@testable import Nola

@MainActor
struct IOSPreparedRunModelTests {
    @Test
    func frozenSkillsRoundTripWithoutCurrentCatalogFallback() throws {
        let original = IOSAgentSkillSnapshot(id: "frozen", name: "Frozen", summary: "summary",
            instructions: "original instructions", version: "1", requiredTools: ["read_file"], validationSteps: ["verify"])
        let encoded = IOSPreparedHostInvocation.skillIdentity([original])
        let decoded = try IOSPreparedHostInvocation.decodeSkillIdentity(encoded)
        #expect(IOSPreparedHostInvocation.skillIdentity(decoded) == encoded)
        #expect(try IOSPreparedHostInvocation.decodeSkillIdentity(.array([])).isEmpty)
        #expect(throws: Error.self) { try IOSPreparedHostInvocation.decodeSkillIdentity(.null) }
        guard case .array(let values) = encoded, case .object(var fields) = values[0] else {
            Issue.record("Missing frozen skill fixture")
            return
        }
        #expect(throws: Error.self) { try IOSPreparedHostInvocation.decodeSkillIdentity(.array(values + values)) }
        fields.removeValue(forKey: "instructions")
        #expect(throws: Error.self) { try IOSPreparedHostInvocation.decodeSkillIdentity(.array([.object(fields)])) }
    }

    @Test
    func oneResolutionOwnsClientModelCapabilityAndParameters() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("application.db"))
        for (tier, effort) in [("concise", "low"), ("balanced", "low"), ("detailed", "high")] {
            let resolved = try runtime.executeAgentCommand(.object([
                "type": .string("resolve_model_reasoning"),
                "declaration": .object(["control": .string("effort"), "efforts": .array([.string("low"), .string("high")])]),
                "protocol": .string("siwc_responses"), "output_limit": .number(16384),
                "supports_reasoning": .bool(true), "requested": .null, "tier": .string(tier)
            ]))
            #expect(resolved.objectFields?["selection"] == .object(["kind": .string("effort"), "value": .string(effort)]))
        }
        let selected = ModelInfo(id: "profile", name: "Selected", source: "api", isLocal: false)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.maxOutputTokens = 64
        parameters.temperature = 0
        parameters.topP = 0
        parameters.seed = 9_007_199_254_740_993
        parameters.stopSequences = "  preserve space  \nsecond"
        var resolutions = 0
        let prepared = try IOSPreparedRunModel.prepare(model: selected, parameters: parameters, resolveCloud: { input in
            resolutions += 1
            #expect(input.id == "profile")
            return IOSResolvedCloudRunProfile(model: ModelInfo(id: input.id, name: "Resolved", source: "api", isLocal: false,
                baseURL: "https://fixture.invalid/v1", apiKey: "test-only",
                upstreamModelId: "resolved-model", wireProtocol: .chatCompletions,
                executionCapabilities: .init(toolCalls: true, reasoning: true, samplingParameters: true)), profileID: input.id, revision: 7,
                parametersJSON: "{}", reasoningSummarySupported: false,
                reasoningSelection: .object(["kind": .string("effort"), "value": .string("high")]), reasoningTier: "detailed")
        }, admitOutput: { route, tokens in
            try IOSPreparedHostOutputAdmission.admit(route: route, outputTokens: tokens, using: runtime)
        })
        parameters.maxOutputTokens = 64000
        #expect(resolutions == 1)
        #expect(prepared.client is OpenAIProvider)
        #expect(prepared.modelName == "resolved-model")
        #expect(prepared.supportsToolCalls)
        #expect(prepared.parameters.maxOutputTokens == 64)
        #expect(prepared.parameters.temperature == 0)
        #expect(prepared.cloudProfile?.profileID == "profile")
        #expect(prepared.cloudProfile?.revision == 7)
        guard case .http(let configuration, let source) = prepared.executionSource else {
            Issue.record("Expected the exact admitted HTTP configuration")
            return
        }
        #expect(configuration.model == prepared.modelName)
        #expect(configuration.baseURL.absoluteString == "https://fixture.invalid/v1")
        #expect(configuration.apiKey == "test-only")
        #expect(configuration.wireProtocol == .chatCompletions)
        #expect(configuration.executionCapabilities.toolCalls == prepared.supportsToolCalls)
        #expect(configuration.maxCompletionTokens == 64)
        #expect(configuration.temperature == 0)
        #expect(configuration.topP == 0)
        #expect(configuration.reasoningSummarySupported == false)
        #expect(configuration.seed == 9_007_199_254_740_993)
        #expect(configuration.stopSequences == ["  preserve space  ", "second"])
        #expect(source.profileID == "profile")
        #expect(source.revision == 7)
        #expect(source.parametersJSON == "{}", "Profile defaults are provenance, not a replacement for frozen session parameters")
        guard case .object(let admission) = prepared.outputAdmission,
              case .object(let output)? = admission["resolved"] else {
            Issue.record("Expected shared output admission")
            return
        }
        #expect(output["tokens"] == .number(64))
        let captured = try prepared.prepareHostInvocation()
        let policy: AgentCoreJSONValue = .object(["permissions": .array([]), "host": .string("ios")])
        let binding = try captured.binding(executionPolicy: policy)
        guard case .object(let fields) = binding,
              case .object(let generation)? = fields["generation"],
              case .object(let options)? = generation["generation_options"],
              case .object(let route)? = fields["primary"] else {
            Issue.record("Missing admitted Host binding")
            return
        }
        #expect(fields["version"] == .number(8))
        #expect(route["siwc_account"] == .null)
        #expect(fields["output_admission"] == prepared.outputAdmission)
        #expect(route["model"] == .string(configuration.model))
        #expect(generation["max_output_tokens"] == .number(64))
        #expect(generation["temperature"] == .number(0))
        #expect(generation["reasoning_tier"] == .string("detailed"))
        #expect(generation["reasoning_selection"] == .object(["kind": .string("effort"), "value": .string("high")]))
        #expect(generation["reasoning_summary_supported"] == .bool(false))
        var unknownConfiguration = configuration
        unknownConfiguration.reasoningSummarySupported = nil
        let unknown = IOSPreparedRunModel(client: OpenAIProvider(config: unknownConfiguration),
            parameters: prepared.parameters, modelName: prepared.modelName, supportsToolCalls: prepared.supportsToolCalls,
            executionSource: .http(configuration: unknownConfiguration, profile: source), outputAdmission: prepared.outputAdmission)
        #expect(try unknown.prepareHostInvocation().generation.objectFields?["reasoning_summary_supported"] == .null)
        #expect(options["seed"] == .signedInteger(9_007_199_254_740_993))
        #expect(options["stop"] == .array([.string("  preserve space  "), .string("second")]))
        let encoded = String(decoding: try JSONEncoder().encode(binding), as: UTF8.self)
        #expect(!encoded.contains("test-only"), "Durable binding must not contain credentials")
        #expect(try captured.binding(executionPolicy: .object(["host": .string("ios"), "permissions": .array([])])) == binding)
        #expect(try captured.binding(executionPolicy: .object(["host": .string("foreign")])) != binding)
    }

    @Test
    func preparationRejectsAdmissionForAnotherExecution() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("application.db"))
        let model = ModelInfo(id: "profile", name: "Fixture", source: "api", isLocal: false,
            baseURL: "https://fixture.invalid/v1", apiKey: "fixture-key", wireProtocol: .chatCompletions)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.maxOutputTokens = 64
        for replaceRoute in [false, true] {
            #expect(throws: Error.self) {
                try IOSPreparedRunModel.prepare(model: model, parameters: parameters,
                    resolveCloud: { _ in .init(model: model, profileID: model.id, revision: 1, parametersJSON: "{}") },
                    admitOutput: { route, tokens in
                        var admittedRoute = route
                        if replaceRoute, case .object(var fields) = route {
                            fields["model"] = .string("another-model")
                            admittedRoute = .object(fields)
                        }
                        return try IOSPreparedHostOutputAdmission.admit(route: admittedRoute,
                            outputTokens: replaceRoute ? tokens : 128, using: runtime)
                    })
            }
        }
    }

    @Test
    func preparationRejectsForeignOrUncommittedSourceIdentity() throws {
        let model = ModelInfo(id: "profile", name: "Fixture", source: "api", isLocal: false,
            baseURL: "https://fixture.invalid/v1", apiKey: "test-only", wireProtocol: .chatCompletions)
        for (id, revision) in [("other-profile", Int64(1)), ("profile", Int64(0))] {
            #expect(throws: Error.self) {
                try IOSPreparedRunModel.prepare(model: model, parameters: .webAlignedCloudDefaults,
                    resolveCloud: { _ in .init(model: model, profileID: id, revision: revision, parametersJSON: "{}") })
            }
        }
    }

    @Test
    func rejectedPreparationNeverFallsBackToSharedClient() throws {
        enum Expected: Error { case unavailable }
        let model = ModelInfo(id: "profile", name: "Selected", source: "api", isLocal: false)
        #expect(throws: Expected.self) {
            try IOSPreparedRunModel.prepare(model: model, parameters: .webAlignedCloudDefaults,
                resolveCloud: { _ in throw Expected.unavailable })
        }
        var invalid = ModelParameters.webAlignedCloudDefaults
        invalid.maxOutputTokens = 0
        #expect(throws: Error.self) {
            try IOSPreparedRunModel.prepare(model: model, parameters: invalid, resolveCloud: { _ in
                Issue.record("Invalid parameters must fail before resolving a provider")
                throw Expected.unavailable
            })
        }
    }
}
