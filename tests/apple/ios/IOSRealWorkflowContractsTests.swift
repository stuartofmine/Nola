// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Dense real-workflow contract & NolaCore FFI integration tests for iOS covering strict 2-mode permissions,
//   AppleModelCatalogDiscovery (/props & OpenRouter), live model connection lifecycle, conversations, backup/restore,
//   experts, agent rooms, memories, automations, draft data source overrides, and non-destructive v1/corrupt DB rejection.

import Foundation
import SQLite3
import XCTest
@testable import Nola

final class IOSRealWorkflowContractsTests: XCTestCase {
    private func makeTempDirectory(_ prefix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-workflow-\(prefix)-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return dir
    }

    private func makeHTTPResponse(url: String, statusCode: Int = 200) throws -> HTTPURLResponse {
        try XCTUnwrap(HTTPURLResponse(
            url: try XCTUnwrap(URL(string: url)),
            statusCode: statusCode,
            httpVersion: "HTTP/1.1",
            headerFields: ["Content-Type": "application/json"]
        ))
    }

    private final class InMemoryCredentialBox: @unchecked Sendable {
        private let lock = NSLock()
        private var storage: [String: Data] = [:]

        func makeOwner() -> IOSCloudCredentialOwner {
            IOSCloudCredentialOwner(
                save: { account, data in
                    self.lock.lock()
                    defer { self.lock.unlock() }
                    guard self.storage[account] == nil else { return false }
                    self.storage[account] = data
                    return true
                },
                read: { account in
                    self.lock.lock()
                    defer { self.lock.unlock() }
                    return self.storage[account]
                },
                delete: { account in
                    self.lock.lock()
                    defer { self.lock.unlock() }
                    self.storage.removeValue(forKey: account)
                }
            )
        }
    }

    private func sampleResolvedTools() -> [AgentCoreResolvedTool] {
        [
            AgentCoreResolvedTool(
                toolId: "noop_tool",
                name: "noop_tool",
                enabled: true,
                route: "device",
                capability: "none_cap",
                permission: "none",
                authorization: "granted",
                risk: "low",
                requiresResultEgress: false,
                reason: "allowed",
                schema: .object([:])
            ),
            AgentCoreResolvedTool(
                toolId: "camera_denied",
                name: "camera_denied",
                enabled: true,
                route: "device",
                capability: "camera",
                permission: "system_permission",
                authorization: "denied",
                risk: "medium",
                requiresResultEgress: false,
                reason: "denied",
                schema: .object([:])
            ),
            AgentCoreResolvedTool(
                toolId: "calendar_read",
                name: "calendar_read",
                enabled: true,
                route: "device",
                capability: "calendar",
                permission: "system_permission",
                authorization: "granted",
                risk: "low",
                requiresResultEgress: false,
                reason: "allowed",
                schema: .object([:])
            ),
            AgentCoreResolvedTool(
                toolId: "location_current",
                name: "location_current",
                enabled: true,
                route: "device",
                capability: "location",
                permission: "system_permission",
                authorization: "ask",
                risk: "medium",
                requiresResultEgress: false,
                reason: "ask",
                schema: .object([:])
            ),
        ]
    }

    // MARK: - 1. Strict 2-Mode IOSPermissionMode & DeviceAgentCoordinator.permissionRules

    func testIOSPermissionModeStrictTwoModeContractAndRejectsAcceptEdits() throws {
        XCTAssertEqual(IOSPermissionMode.allCases, [.cautious, .allowAll])
        XCTAssertEqual(IOSPermissionMode(rawValue: "cautious"), .cautious)
        XCTAssertEqual(IOSPermissionMode(rawValue: "allow_all"), .allowAll)
        XCTAssertNil(IOSPermissionMode(rawValue: "accept_edits"))
        XCTAssertNil(IOSPermissionMode(rawValue: "ACCEPT_EDITS"))
        XCTAssertNil(IOSPermissionMode(rawValue: "plan"))
        XCTAssertNil(IOSPermissionMode(rawValue: ""))

        XCTAssertEqual(IOSPermissionMode.cautious.id, "cautious")
        XCTAssertEqual(IOSPermissionMode.allowAll.id, "allow_all")
        XCTAssertEqual(IOSPermissionMode.cautious.networkPolicy, "ask")
        XCTAssertEqual(IOSPermissionMode.allowAll.networkPolicy, "allow")
        XCTAssertTrue(IOSPermissionMode.cautious.autoApprove)
        XCTAssertTrue(IOSPermissionMode.allowAll.autoApprove)
        XCTAssertFalse(IOSPermissionMode.cautious.title.isEmpty)
        XCTAssertFalse(IOSPermissionMode.allowAll.detail.isEmpty)

        XCTAssertThrowsError(try JSONDecoder().decode(IOSPermissionMode.self, from: Data(#""accept_edits""#.utf8)))
        XCTAssertEqual(
            try JSONDecoder().decode(IOSPermissionMode.self, from: Data(#""allow_all""#.utf8)),
            .allowAll
        )
    }

    func testDeviceAgentCoordinatorPermissionRulesCautiousWithHostToolsAndScopedGrants() throws {
        let grants = [
            AgentCoreScopedGrant(capability: "files_read", route: "device", scope: "  file:///Documents/Notes  ", expiresAtMs: 9_999_999_999),
            AgentCoreScopedGrant(capability: "files_write", route: "device", scope: "   ", expiresAtMs: 9_999_999_999),
            AgentCoreScopedGrant(capability: "photos", route: "device", scope: nil, expiresAtMs: 9_999_999_999),
        ]

        let result = DeviceAgentCoordinator.permissionRules(
            enabledTools: sampleResolvedTools(),
            scopedGrants: grants,
            permissionMode: .cautious
        )
        guard case .object(let root) = result, case .array(let rules)? = root["rules"] else {
            return XCTFail("Expected object with rules array, got \(result)")
        }
        XCTAssertEqual(rules.count, 4)
        XCTAssertEqual(rules[0], .object([
            "capability_pattern": .string("*"),
            "resource_pattern": .string("*"),
            "decision": .string("allow"),
            "source": .string("product_default"),
        ]))
        XCTAssertEqual(rules[1], .object([
            "capability_pattern": .string("calendar"),
            "resource_pattern": .string("*"),
            "decision": .string("allow"),
            "source": .string("host"),
        ]))
        XCTAssertEqual(rules[2], .object([
            "capability_pattern": .string("location"),
            "resource_pattern": .string("*"),
            "decision": .string("ask"),
            "source": .string("host"),
        ]))
        XCTAssertEqual(rules[3], .object([
            "capability_pattern": .string("files_read"),
            "resource_pattern": .string("file:///Documents/Notes"),
            "decision": .string("allow"),
            "source": .string("host"),
        ]))
    }

    func testDeviceAgentCoordinatorPermissionRulesAllowAllComposesInvocationPolicyWithHostAndScopedGrants() throws {
        let grants = [
            AgentCoreScopedGrant(capability: "files_read", route: "device", scope: "/workspace/report.md", expiresAtMs: 9_999_999_999),
        ]

        let result = DeviceAgentCoordinator.permissionRules(
            enabledTools: sampleResolvedTools(),
            scopedGrants: grants,
            permissionMode: .allowAll
        )
        guard case .object(let root) = result, case .array(let rules)? = root["rules"] else {
            return XCTFail("Expected object with rules array, got \(result)")
        }
        XCTAssertEqual(rules.count, 4)
        XCTAssertEqual(rules[0], .object([
            "capability_pattern": .string("*"),
            "resource_pattern": .string("*"),
            "decision": .string("allow"),
            "source": .string("invocation_policy"),
        ]))
        XCTAssertEqual(rules[1], .object([
            "capability_pattern": .string("calendar"),
            "resource_pattern": .string("*"),
            "decision": .string("allow"),
            "source": .string("host"),
        ]))
        XCTAssertEqual(rules[2], .object([
            "capability_pattern": .string("location"),
            "resource_pattern": .string("*"),
            "decision": .string("ask"),
            "source": .string("host"),
        ]))
        XCTAssertEqual(rules[3], .object([
            "capability_pattern": .string("files_read"),
            "resource_pattern": .string("/workspace/report.md"),
            "decision": .string("allow"),
            "source": .string("host"),
        ]))
    }

    // MARK: - 2. Full AppleModelCatalogDiscovery Matrix

    func testPropsRequestURLNormalizationAndBearerAuth() throws {
        let cases: [(endpoint: String, expectedURL: String)] = [
            ("http://192.168.1.50:8080/v1", "http://192.168.1.50:8080/props"),
            ("http://192.168.1.50:8080/v1/models", "http://192.168.1.50:8080/props"),
            ("https://gateway.example.invalid/api/v1", "https://gateway.example.invalid/api/props"),
            ("https://gateway.example.invalid/api/v1/models?extra=1", "https://gateway.example.invalid/api/props"),
            ("http://127.0.0.1:8080", "http://127.0.0.1:8080/props"),
            ("http://127.0.0.1:8080/", "http://127.0.0.1:8080/props"),
        ]

        for (endpoint, expectedURL) in cases {
            let req = try XCTUnwrap(AppleModelCatalogDiscovery.propsRequest(endpoint: endpoint, secret: "sk-lan-token"))
            XCTAssertEqual(req.url?.absoluteString, expectedURL, "Unexpected /props URL for \(endpoint)")
            XCTAssertEqual(req.value(forHTTPHeaderField: "Authorization"), "Bearer sk-lan-token")
            XCTAssertEqual(req.timeoutInterval, 3, accuracy: 0.001)
        }

        let anonymousReq = try XCTUnwrap(AppleModelCatalogDiscovery.propsRequest(endpoint: "http://127.0.0.1:8080/v1", secret: ""))
        XCTAssertNil(anonymousReq.value(forHTTPHeaderField: "Authorization"))
    }

    func testEnrichWithPropsStrataLlamaServerPreserveReasoningVisionAudioAndLimits() throws {
        let base = [
            AppleModelCatalogDiscovery.Model(id: "strata-qwen3-vl", displayName: "Strata Qwen3 VL")
        ]
        let propsJSON = #"""
        {
          "model_alias": "strata-qwen3-vl",
          "chat_template_caps": {
            "supports_preserve_reasoning": true,
            "supports_tool_calls": true
          },
          "modalities": {
            "vision": true,
            "audio": true
          },
          "default_generation_settings": {
            "n_ctx": 131072,
            "params": {
              "n_predict": 8192
            }
          }
        }
        """#
        let resp = try makeHTTPResponse(url: "http://192.168.1.50:8080/props", statusCode: 200)
        let enriched = AppleModelCatalogDiscovery.enrichWithProps(models: base, data: Data(propsJSON.utf8), response: resp)
        XCTAssertEqual(enriched.count, 1)
        let model = enriched[0]
        XCTAssertEqual(model.reasoningSupported, true)
        XCTAssertEqual(model.reasoningLevels, ["none", "low", "medium", "high"])
        XCTAssertEqual(model.toolCalls, true)
        XCTAssertEqual(model.modalities, ["text", "image", "audio"])
        XCTAssertEqual(model.contextWindow, 131_072)
        XCTAssertEqual(model.outputLimit, 8_192)
    }

    func testEnrichWithPropsChatTemplateEnableThinkingAndFalsePreserveReasoning() throws {
        let base = [AppleModelCatalogDiscovery.Model(id: "qwen3-local", displayName: "Qwen3 Local")]
        let resp = try makeHTTPResponse(url: "http://127.0.0.1:8080/props", statusCode: 200)

        let templateProps = #"""
        {
          "chat_template": "{% if enable_thinking %}<think>{% endif %}",
          "chat_template_caps": {
            "supports_tools": true
          }
        }
        """#
        let enrichedFromTemplate = AppleModelCatalogDiscovery.enrichWithProps(
            models: base, data: Data(templateProps.utf8), response: resp
        )
        XCTAssertEqual(enrichedFromTemplate[0].reasoningSupported, true)
        XCTAssertEqual(enrichedFromTemplate[0].reasoningLevels, ["none", "low", "medium", "high"])
        XCTAssertEqual(enrichedFromTemplate[0].toolCalls, true)

        let disabledProps = #"""
        {
          "chat_template": "{% if enable_thinking %}<think>{% endif %}",
          "chat_template_caps": {
            "supports_preserve_reasoning": false
          }
        }
        """#
        let enrichedDisabled = AppleModelCatalogDiscovery.enrichWithProps(
            models: base, data: Data(disabledProps.utf8), response: resp
        )
        XCTAssertEqual(enrichedDisabled[0].reasoningSupported, false)
        XCTAssertTrue(enrichedDisabled[0].reasoningLevels.isEmpty)
    }

    func testEnrichWithPropsMultiModelAliasMatchingPreservesExistingFieldsAndGracefulNoOps() throws {
        let existingLevelsModel = AppleModelCatalogDiscovery.Model(
            id: "coder-model",
            displayName: "Coder",
            reasoningLevels: ["low", "xhigh"],
            contextWindow: 64_000,
            outputLimit: 4_096,
            modalities: ["text"],
            reasoningSupported: true,
            toolCalls: false
        )
        let targetModel = AppleModelCatalogDiscovery.Model(id: "qwen-thinker", displayName: "Qwen Thinker")
        let models = [existingLevelsModel, targetModel]

        // Case-insensitive model_alias matches ONLY qwen-thinker
        let aliasProps = #"""
        {
          "model_alias": "QWEN-THINKER",
          "chat_template_caps": { "supports_preserve_reasoning": true, "supports_tool_calls": true },
          "modalities": { "vision": true },
          "default_generation_settings": { "n_ctx": 65536, "params": { "n_predict": 4096 } }
        }
        """#
        let okResp = try makeHTTPResponse(url: "http://127.0.0.1:8080/props", statusCode: 200)
        let enriched = AppleModelCatalogDiscovery.enrichWithProps(models: models, data: Data(aliasProps.utf8), response: okResp)
        XCTAssertEqual(enriched[0], existingLevelsModel, "Non-matching model must stay completely untouched")
        XCTAssertEqual(enriched[1].reasoningSupported, true)
        XCTAssertEqual(enriched[1].reasoningLevels, ["none", "low", "medium", "high"])
        XCTAssertEqual(enriched[1].modalities, ["text", "image"])
        XCTAssertEqual(enriched[1].contextWindow, 65_536)
        XCTAssertEqual(enriched[1].outputLimit, 4_096)

        // Single-model enrichment must NEVER overwrite pre-existing limits/levels/toolCalls
        let singlePreserved = AppleModelCatalogDiscovery.enrichWithProps(
            models: [existingLevelsModel], data: Data(aliasProps.utf8), response: okResp
        )[0]
        XCTAssertEqual(singlePreserved.reasoningLevels, ["low", "xhigh"])
        XCTAssertEqual(singlePreserved.contextWindow, 64_000)
        XCTAssertEqual(singlePreserved.outputLimit, 4_096)
        XCTAssertEqual(singlePreserved.toolCalls, false)

        // Non-200 response -> graceful no-op
        let errResp = try makeHTTPResponse(url: "http://127.0.0.1:8080/props", statusCode: 404)
        XCTAssertEqual(
            AppleModelCatalogDiscovery.enrichWithProps(models: models, data: Data(aliasProps.utf8), response: errResp),
            models
        )

        // Malformed JSON -> graceful no-op
        XCTAssertEqual(
            AppleModelCatalogDiscovery.enrichWithProps(models: models, data: Data("{not-valid-json".utf8), response: okResp),
            models
        )
    }

    func testParseOpenRouterSupportedParametersAndExplicitReasoningLevelsPreservation() throws {
        let catalogJSON = #"""
        {
          "data": [
            {
              "id": "openrouter/reasoner-a",
              "name": "Reasoner A",
              "context_length": 128000,
              "top_provider": { "max_completion_tokens": 16384 },
              "architecture": { "input_modalities": ["text", "image"] },
              "supported_parameters": ["reasoning", "tools", "temperature"]
            },
            {
              "id": "openrouter/reasoner-b",
              "display_name": "Reasoner B",
              "supported_parameters": ["include_reasoning"]
            },
            {
              "id": "openrouter/reasoner-c",
              "supported_parameters": ["reasoning_effort", "tools"],
              "supported_reasoning_levels": [
                {"effort": "minimal"},
                {"effort": "high"},
                {"effort": "minimal"}
              ]
            },
            {
              "id": "openrouter/plain-chat",
              "supported_parameters": ["temperature"]
            }
          ]
        }
        """#
        let resp = try makeHTTPResponse(url: "https://openrouter.ai/api/v1/models", statusCode: 200)
        let parsed = try AppleModelCatalogDiscovery.parse(data: Data(catalogJSON.utf8), response: resp)
        XCTAssertEqual(parsed.count, 4)

        let a = parsed[0]
        XCTAssertEqual(a.displayName, "Reasoner A")
        XCTAssertEqual(a.contextWindow, 128_000)
        XCTAssertEqual(a.outputLimit, 16_384)
        XCTAssertEqual(a.modalities, ["text", "image"])
        XCTAssertEqual(a.reasoningSupported, true)
        XCTAssertEqual(a.reasoningLevels, ["none", "low", "medium", "high"])
        XCTAssertEqual(a.toolCalls, true)
        XCTAssertEqual(a.samplingParameters, true)

        let b = parsed[1]
        XCTAssertEqual(b.reasoningSupported, true)
        XCTAssertEqual(b.reasoningLevels, ["none", "low", "medium", "high"])
        XCTAssertEqual(b.toolCalls, false)
        XCTAssertEqual(b.samplingParameters, false)

        // Explicit supported_reasoning_levels must be preserved & deduplicated, NEVER overwritten!
        let c = parsed[2]
        XCTAssertEqual(c.reasoningSupported, true)
        XCTAssertEqual(c.reasoningLevels, ["minimal", "high"])
        XCTAssertEqual(c.toolCalls, true)

        let plain = parsed[3]
        XCTAssertNil(plain.reasoningSupported)
        XCTAssertTrue(plain.reasoningLevels.isEmpty)
        XCTAssertEqual(plain.toolCalls, false)
        XCTAssertEqual(plain.samplingParameters, true)
    }

    func testDiscoveryErrorDescriptionsAndLocalNetworkProhibitedMessage() throws {
        let localNet = AppleModelCatalogDiscovery.DiscoveryError.localNetworkProhibited("192.168.1.50")
        let desc = try XCTUnwrap(localNet.errorDescription)
        XCTAssertTrue(desc.contains("192.168.1.50"))
        XCTAssertTrue(desc.localizedCaseInsensitiveContains("local network prohibited"))
        XCTAssertTrue(desc.contains("Local Network"))

        let unauthResp = try makeHTTPResponse(url: "https://api.example.invalid/v1/models", statusCode: 401)
        let errPayload = Data(#"{"error":{"message":"Invalid API key provided"}}"#.utf8)
        XCTAssertThrowsError(try AppleModelCatalogDiscovery.parse(data: errPayload, response: unauthResp)) { error in
            XCTAssertTrue(error.localizedDescription.contains("401"))
            XCTAssertTrue(error.localizedDescription.contains("Invalid API key provided"))
        }

        let okResp = try makeHTTPResponse(url: "https://api.example.invalid/v1/models", statusCode: 200)
        XCTAssertThrowsError(try AppleModelCatalogDiscovery.parse(data: Data(#"{"models":[]}"#.utf8), response: okResp))
    }

    // MARK: - 3. Real NolaCore Model Connection Lifecycle, /props Refresh, Overrides & Deletion

    func testModelConnectionCreateRefreshEnableRediscoverWithPropsImmediatelyUpdatesReasoning() throws {
        let root = try makeTempDirectory("model-props-flow")
        let db = root.appendingPathComponent("model-props.db")
        let runtime = try IOSApplicationRuntime(databaseURL: db)
        let creds = InMemoryCredentialBox().makeOwner()
        let service = IOSModelConnectionService(application: runtime, credentials: creds)
        let endpoint = "http://192.168.1.50:8080/v1"
        let t0 = Int64(Date().timeIntervalSince1970 * 1_000)

        // 1. Create connection with plain model (no reasoning)
        let created = try service.create(
            name: "Strata Local",
            endpoint: endpoint,
            wireProtocol: "chat_completions",
            secret: "sk-local-props",
            modelID: "strata-qwen3"
        )

        // 2. Refresh catalog WITHOUT reasoning and enable model
        let plainModel = AppleModelCatalogDiscovery.Model(
            id: "strata-qwen3",
            displayName: "Strata Qwen3",
            modalities: ["text"],
            reasoningSupported: false,
            toolCalls: true,
            samplingParameters: true
        )
        let plainEntry = try JSONDecoder().decode(AgentCoreJSONValue.self, from: plainModel.catalogData(wireProtocol: "chat_completions"))
        let refreshedPlain: IOSModelConnectionSnapshot = try service.command([
            "type": .string("refresh_connection_catalog"),
            "connection_id": .string(created.id),
            "expected_revision": .signedInteger(created.revision),
            "models": .array([plainEntry]),
            "observed_at_ms": .signedInteger(t0),
        ])
        try service.enable(refreshedPlain, modelIDs: ["chat_completions:strata-qwen3"])

        let initialProfile = try XCTUnwrap(try runtime.listModelProfiles().first(where: { $0.modelName == "strata-qwen3" }))
        let initialSrc = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(initialProfile.sourceJSON.utf8)) as? [String: Any])
        XCTAssertNil(initialSrc["reasoning"], "Initial profile must not expose reasoning before /props enrichment")

        // 3. Re-discover with /props enrichment and refresh catalog (WITHOUT re-toggling enable)
        let propsData = Data(#"""
        {
          "chat_template_caps": { "supports_preserve_reasoning": true, "supports_tool_calls": true },
          "modalities": { "vision": true },
          "default_generation_settings": { "n_ctx": 131072, "params": { "n_predict": 8192 } }
        }
        """#.utf8)
        let propsResp = try makeHTTPResponse(url: "http://192.168.1.50:8080/props", statusCode: 200)
        let enriched = AppleModelCatalogDiscovery.enrichWithProps(
            models: [AppleModelCatalogDiscovery.Model(id: "strata-qwen3", displayName: "Strata Qwen3")],
            data: propsData,
            response: propsResp
        )
        let enrichedEntry = try JSONDecoder().decode(AgentCoreJSONValue.self, from: enriched[0].catalogData(wireProtocol: "chat_completions"))
        let _: IOSModelConnectionSnapshot = try service.command([
            "type": .string("refresh_connection_catalog"),
            "connection_id": .string(created.id),
            "expected_revision": .signedInteger(refreshedPlain.revision),
            "models": .array([enrichedEntry]),
            "observed_at_ms": .signedInteger(t0 + 5_000),
        ])

        // 4. Immediately verify listModelProfiles() exposes reasoning efforts WITHOUT calling service.enable again!
        let updatedProfile = try XCTUnwrap(try runtime.listModelProfiles().first(where: { $0.modelName == "strata-qwen3" }))
        let updatedSrc = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(updatedProfile.sourceJSON.utf8)) as? [String: Any])
        let reasoning = try XCTUnwrap(updatedSrc["reasoning"] as? [String: Any])
        XCTAssertEqual(reasoning["control"] as? String, "effort")
        XCTAssertEqual(reasoning["efforts"] as? [String], ["none", "low", "medium", "high"])
        let execCaps = try XCTUnwrap(updatedSrc["executionCapabilities"] as? [String: Any])
        XCTAssertEqual(execCaps["reasoning"] as? Bool, true)
        XCTAssertEqual(execCaps["imageInput"] as? Bool, true)
    }

    func testModelCapabilityOverrideToggleAndDeleteConnectionCleanUpProfilesAcrossColdReopen() throws {
        let root = try makeTempDirectory("model-override-delete")
        let db = root.appendingPathComponent("override-delete.db")
        let creds = InMemoryCredentialBox().makeOwner()
        let endpoint = "https://override.example.invalid/v1"
        let connID: String

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: db)
            let service = IOSModelConnectionService(application: runtime, credentials: creds)
            let reasoningModel = AppleModelCatalogDiscovery.Model(
                id: "reasoner-v2",
                displayName: "Reasoner V2",
                reasoningLevels: ["none", "low", "medium", "high"],
                contextWindow: 128_000,
                outputLimit: 8_192,
                modalities: ["text"],
                reasoningSupported: true,
                toolCalls: true,
                samplingParameters: true
            )
            let created = try service.create(
                name: "Override Gateway",
                endpoint: endpoint,
                wireProtocol: "chat_completions",
                secret: "sk-override-test",
                modelID: "reasoner-v2",
                discoveredModel: reasoningModel
            )
            connID = created.id
            try service.enable(created, modelIDs: ["chat_completions:reasoner-v2"])

            let stored = try XCTUnwrap(try runtime.listModelProfiles().first(where: { $0.modelName == "reasoner-v2" }))
            let modelInfo = try XCTUnwrap(ModelSelectionManager.modelInfo(from: stored))

            // Toggle reasoning OFF via ModelSelectionManager.updateExecutionCapabilities
            let offModel = try ModelSelectionManager.updateExecutionCapabilities(
                for: modelInfo,
                to: ModelExecutionCapabilities(toolCalls: true, imageInput: false, reasoning: false, samplingParameters: true),
                using: runtime
            )
            XCTAssertEqual(offModel.executionCapabilities?.reasoning, false)
            let offProfile = try XCTUnwrap(try runtime.listModelProfiles().first(where: { $0.id == stored.id }))
            let offSrc = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(offProfile.sourceJSON.utf8)) as? [String: Any])
            XCTAssertNil(offSrc["reasoning"])

            // Toggle reasoning back ON + vision ON
            let onModel = try ModelSelectionManager.updateExecutionCapabilities(
                for: offModel,
                to: ModelExecutionCapabilities(toolCalls: true, imageInput: true, reasoning: true, samplingParameters: true),
                using: runtime
            )
            XCTAssertEqual(onModel.executionCapabilities?.reasoning, true)
            XCTAssertEqual(onModel.executionCapabilities?.imageInput, true)
            let onProfile = try XCTUnwrap(try runtime.listModelProfiles().first(where: { $0.id == stored.id }))
            let onSrc = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(onProfile.sourceJSON.utf8)) as? [String: Any])
            let onReasoning = try XCTUnwrap(onSrc["reasoning"] as? [String: Any])
            XCTAssertEqual(onReasoning["efforts"] as? [String], ["none", "low", "medium", "high"])

            // Delete connection and verify linked profiles are cleaned up
            let deleted: AgentCoreJSONValue = try service.command([
                "type": .string("delete_connection"),
                "id": .string(connID),
                "delete_account": .bool(true),
            ])
            XCTAssertEqual(deleted.objectFields?["ok"], .bool(true))
            XCTAssertEqual(deleted.objectFields?["deleted"], .bool(true))
            XCTAssertFalse(try service.connections().contains(where: { $0.id == connID }))
            XCTAssertTrue(try runtime.listModelProfiles().filter { $0.modelName == "reasoner-v2" }.isEmpty)
        }

        // Cold reopen confirms connection & profiles remain gone
        let reopened = try IOSApplicationRuntime(databaseURL: db)
        let reopenedService = IOSModelConnectionService(application: reopened, credentials: creds)
        XCTAssertFalse(try reopenedService.connections().contains(where: { $0.id == connID }))
        XCTAssertTrue(try reopened.listModelProfiles().isEmpty)
    }

    func testListModelProfilesReadTimeOverlayHydratesStaleProfileFromFresherCatalog() throws {
        let root = try makeTempDirectory("model-read-overlay")
        let db = root.appendingPathComponent("overlay.db")
        let runtime = try IOSApplicationRuntime(databaseURL: db)
        let creds = InMemoryCredentialBox().makeOwner()
        let service = IOSModelConnectionService(application: runtime, credentials: creds)
        let staleTs: Int64 = 1_787_600_000_000
        let freshTs: Int64 = staleTs + 60_000

        let freshModel = AppleModelCatalogDiscovery.Model(
            id: "o-reasoner-ios",
            displayName: "O-Reasoner iOS",
            reasoningLevels: ["low", "medium", "high"],
            contextWindow: 200_000,
            outputLimit: 16_384,
            modalities: ["text"],
            reasoningSupported: true,
            summarySupported: true,
            toolCalls: true,
            samplingParameters: false
        )
        let created = try service.create(
            name: "Overlay Gateway iOS",
            endpoint: "https://overlay.example.invalid/v1",
            wireProtocol: "responses",
            secret: "sk-overlay-ios",
            modelID: "o-reasoner-ios",
            discoveredModel: freshModel
        )
        let entry = try JSONDecoder().decode(AgentCoreJSONValue.self, from: freshModel.catalogData(wireProtocol: "responses"))
        let _: IOSModelConnectionSnapshot = try service.command([
            "type": .string("refresh_connection_catalog"),
            "connection_id": .string(created.id),
            "expected_revision": .signedInteger(created.revision),
            "models": .array([entry]),
            "observed_at_ms": .signedInteger(freshTs),
        ])

        // Insert a stale profile snapshot missing reasoning and token limits, linked to the fresher connection
        let staleProfileID = "stale-overlay:\(created.id):responses:o-reasoner-ios"
        try runtime.upsertModelProfile(IOSCanonicalModelProfile(
            id: staleProfileID,
            displayName: "O-Reasoner iOS",
            provider: "openai-compatible",
            modelName: "o-reasoner-ios",
            runtime: "remote",
            sourceJSON: #"{"baseURL":"https://overlay.example.invalid/v1","wireProtocol":"responses","connectionID":"\#(created.id)","catalogKey":"responses:o-reasoner-ios"}"#,
            parametersJSON: "{}",
            selected: false,
            enabled: true,
            revision: 1,
            createdAtMs: staleTs,
            updatedAtMs: staleTs,
            deletedAtMs: nil
        ))

        let overlaid = try XCTUnwrap(try runtime.listModelProfiles().first(where: { $0.id == staleProfileID }))
        let src = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(overlaid.sourceJSON.utf8)) as? [String: Any])
        let reasoning = try XCTUnwrap(src["reasoning"] as? [String: Any])
        XCTAssertEqual(reasoning["efforts"] as? [String], ["low", "medium", "high"])
        XCTAssertEqual(reasoning["responses"] as? Bool, true)
        XCTAssertEqual(src["reasoningSummarySupported"] as? Bool, true)

        let params = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(overlaid.parametersJSON.utf8)) as? [String: Any])
        XCTAssertEqual(params["maxContextLength"] as? Int64, 200_000)
        XCTAssertEqual(params["maxOutputTokens"] as? Int64, 16_384)
    }

    // MARK: - 4. Multi-Conversation Lifecycle, Windowing, Backup/Restore & Cold Reopen

    func testMultiConversationLifecycleRenamePinWindowExportAndColdReopen() throws {
        let root = try makeTempDirectory("multi-conv")
        let db = root.appendingPathComponent("multi-conv.db")
        let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)
        let conv1ID: UUID
        let conv2ID: UUID

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: db)
            let store = try LocalConversationStore(applicationRuntime: runtime)
            conv1ID = try store.createConversation(title: "Release Checklist", language: "en").id
            conv2ID = try store.createConversation(title: "Travel Notes", language: "en").id

            var messages: [ConversationMessage] = []
            for turn in 1...3 {
                messages.append(ConversationMessage(conversationId: conv1ID, role: .user, text: "Question \(turn): verify SQLite WAL"))
                messages.append(ConversationMessage(conversationId: conv1ID, role: .assistant, text: "Answer \(turn): WAL verified"))
            }
            try store.importMessages(conversationId: conv1ID, messages: messages, source: .externalArchive)

            // Rename & pin conv1 (plus verify CAS conflict on stale revision)
            try runtime.renameConversation(id: conv1ID.uuidString, title: "Release 2.0 Checklist", updatedAtMs: nowMs + 100)
            let loaded1 = try XCTUnwrap(try runtime.loadConversation(id: conv1ID.uuidString))
            let cfgRev = max(loaded1.configurationRevision ?? 1, 1)
            let patchRes = try runtime.patchConversationConfiguration(
                id: conv1ID.uuidString,
                expectedRevision: cfgRev,
                patchJSON: #"{"isPinned":true}"#,
                updatedAtMs: nowMs + 200
            )
            XCTAssertEqual(patchRes.status, .committed)
            try patchRes.requireCommitted()

            let stalePatch = try runtime.patchConversationConfiguration(
                id: conv1ID.uuidString,
                expectedRevision: cfgRev,
                patchJSON: #"{"isPinned":false}"#,
                updatedAtMs: nowMs + 210
            )
            XCTAssertEqual(stalePatch.status, .conflict)

            // Paginated window loading
            let tail = try runtime.loadConversationProjectionWindowV1(
                id: conv1ID.uuidString, beforeOrdinal: nil, limit: 2, contentByteLimit: 65_536
            )
            XCTAssertEqual(tail.totalMessageCount, 6)
            XCTAssertEqual(tail.projection.messages.count, 2)
            XCTAssertTrue(tail.hasMore)

            let earlier = try runtime.loadConversationProjectionWindowV1(
                id: conv1ID.uuidString, beforeOrdinal: tail.beforeOrdinal, limit: 2, contentByteLimit: 65_536
            )
            XCTAssertEqual(earlier.projection.messages.count, 2)

            // Activity search filtering
            let searched = try IOSWorkspaceReadFacts.activity(
                runtime: runtime,
                query: AppleActivityQuery(source: nil, needs_response: false, query: "Release 2.0", cursor: nil, limit: 20)
            )
            XCTAssertTrue(searched.entries.contains(where: { $0.conversation_id == conv1ID.uuidString }))
            XCTAssertFalse(searched.entries.contains(where: { $0.conversation_id == conv2ID.uuidString }))
        }

        // Cold reopen verifies both conversations & messages persisted
        let reopened = try IOSApplicationRuntime(databaseURL: db)
        let listed = try reopened.listConversations()
        XCTAssertTrue(listed.contains(where: { $0.id == conv1ID.uuidString && $0.title == "Release 2.0 Checklist" }))
        XCTAssertTrue(listed.contains(where: { $0.id == conv2ID.uuidString && $0.title == "Travel Notes" }))
        let proj = try reopened.loadConversationProjectionV1(id: conv1ID.uuidString)
        XCTAssertEqual(proj.messages.count, 6)

        XCTAssertTrue(try reopened.deleteConversation(id: conv2ID.uuidString))
        XCTAssertNil(try reopened.loadConversation(id: conv2ID.uuidString))
    }

    func testDatabaseBackupArchiveRestoreAndColdReopenVerification() throws {
        let root = try makeTempDirectory("backup-restore")
        let sourceDB = root.appendingPathComponent("source.db")
        let backupDB = root.appendingPathComponent("backup.db")
        let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: sourceDB)
            try runtime.createConversation(
                id: "conv-ios-backup",
                title: "Critical Backup Thread",
                settingsJSON: nil,
                createdAtMs: nowMs,
                updatedAtMs: nowMs
            )
            let memoryStore = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
            _ = try memoryStore.saveMemoryText(
                longTermMemory: "Preserve backup memory across restore.",
                userPreferences: "Use strict verification.",
                expected: memoryStore.loadProfile()
            )

            // Exercise SQLite backup command
            let backupResp = try runtime.executeAgentCommand(.object([
                "type": .string("backup"),
                "destination": .string(backupDB.path),
            ]))
            XCTAssertEqual(backupResp.objectFields?["backup"]?.stringScalar, backupDB.path)

            // Exercise archive manifest + restore revision state machine
            let staged = try runtime.executeAgentCommand(.object([
                "type": .string("stage_archive_manifest"),
                "entries": .array([]),
            ]))
            XCTAssertEqual(staged.objectFields?["staged"], .bool(true))

            let begun = try runtime.executeAgentCommand(.object([
                "type": .string("begin_archive_restore"),
            ]))
            let restoreID = try XCTUnwrap(begun.objectFields?["restore_id"]?.stringScalar)
            let committed = try runtime.executeAgentCommand(.object([
                "type": .string("commit_archive_restore"),
                "restore_id": .string(restoreID),
            ]))
            XCTAssertEqual(committed.objectFields?["status"]?.stringScalar, "committed")
        }

        // Cold open the backup DB directly and verify all data survived
        XCTAssertTrue(FileManager.default.fileExists(atPath: backupDB.path))
        let restoredRuntime = try IOSApplicationRuntime(databaseURL: backupDB)
        let restoredConv = try XCTUnwrap(try restoredRuntime.loadConversation(id: "conv-ios-backup"))
        XCTAssertEqual(restoredConv.title, "Critical Backup Thread")
        let restoredMemory = CanonicalChatMemoryProfileStore(applicationRuntime: restoredRuntime).loadProfile()
        XCTAssertEqual(restoredMemory.longTermMemory, "Preserve backup memory across restore.")
    }

    // MARK: - 5. ExpertPeer, AgentRoom, Memory, Automation & Draft DataSource Overrides

    func testExpertPeerAndAgentRoomLifecycleWithSubmissionsAcrossColdReopen() throws {
        let root = try makeTempDirectory("expert-room")
        let db = root.appendingPathComponent("expert-room.db")
        let roomID = "room-ios-review"
        let submissionID: String

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: db)

            // Create & update ExpertPeer
            let createdExpert = try runtime.executeAgentCommand(.object([
                "type": .string("agent_room"),
                "command": .object([
                    "type": .string("create_expert"),
                    "id": .string("expert-swift"),
                    "name": .string("Swift Architect"),
                    "role": .string("reviewer"),
                    "system_prompt": .string("Review concurrency and Sendable isolation."),
                    "model_override": .string("claude-sonnet-4-5"),
                ]),
            ]))
            XCTAssertEqual(createdExpert.objectFields?["ok"], .bool(true))

            let listedExperts = try runtime.executeAgentCommand(.object([
                "type": .string("agent_room"),
                "command": .object(["type": .string("list_experts")]),
            ]))
            guard case .array(let experts)? = listedExperts.objectFields?["experts"] else {
                return XCTFail("Expected experts array")
            }
            XCTAssertTrue(experts.contains(where: { $0.objectFields?["id"]?.stringScalar == "expert-swift" }))

            // Create AgentRoom & drive submission lifecycle
            let createdRoom = try runtime.executeAgentCommand(.object([
                "type": .string("agent_room"),
                "command": .object([
                    "type": .string("create_room"),
                    "id": .string(roomID),
                    "name": .string("iOS Architecture Room"),
                    "participant_expert_ids": .array([.string("expert-swift")]),
                ]),
            ]))
            XCTAssertEqual(createdRoom.objectFields?["room"]?.objectFields?["id"]?.stringScalar, roomID)

            let submitted = try runtime.executeAgentCommand(.object([
                "type": .string("agent_room"),
                "command": .object([
                    "type": .string("submit"),
                    "room_id": .string(roomID),
                    "content": .string("Audit IOSModelConnectionService actor boundaries"),
                ]),
            ]))
            submissionID = try XCTUnwrap(submitted.objectFields?["submission_id"]?.stringScalar)

            let selected = try runtime.executeAgentCommand(.object([
                "type": .string("agent_room"),
                "command": .object([
                    "type": .string("select_recipient"),
                    "room_id": .string(roomID),
                    "submission_id": .string(submissionID),
                    "recipient_id": .string("expert-swift"),
                ]),
            ]))
            XCTAssertEqual(selected.objectFields?["recipient_id"]?.stringScalar, "expert-swift")

            let reconciled = try runtime.executeAgentCommand(.object([
                "type": .string("agent_room"),
                "command": .object([
                    "type": .string("reconcile_execution"),
                    "submission_id": .string(submissionID),
                    "status": .string("completed"),
                ]),
            ]))
            XCTAssertEqual(reconciled.objectFields?["reconciled"], .bool(true))
        }

        // Cold reopen confirms room, expert & submission persisted, then clean up
        let reopened = try IOSApplicationRuntime(databaseURL: db)
        let room = try reopened.executeAgentCommand(.object([
            "type": .string("agent_room"),
            "command": .object(["type": .string("get_room"), "id": .string(roomID)]),
        ]))
        XCTAssertEqual(room.objectFields?["name"]?.stringScalar, "iOS Architecture Room")

        let subs = try reopened.executeAgentCommand(.object([
            "type": .string("agent_room"),
            "command": .object(["type": .string("submissions"), "room_id": .string(roomID)]),
        ]))
        guard case .array(let subArray) = subs else { return XCTFail("Expected submissions array") }
        XCTAssertEqual(subArray.count, 1)
        XCTAssertEqual(subArray[0].objectFields?["submission_id"]?.stringScalar, submissionID)

        let deletedRoom = try reopened.executeAgentCommand(.object([
            "type": .string("agent_room"),
            "command": .object(["type": .string("delete_room"), "id": .string(roomID)]),
        ]))
        XCTAssertEqual(deletedRoom.objectFields?["deleted"], .bool(true))

        let deletedExpert = try reopened.executeAgentCommand(.object([
            "type": .string("agent_room"),
            "command": .object(["type": .string("delete_expert"), "id": .string("expert-swift")]),
        ]))
        XCTAssertEqual(deletedExpert.objectFields?["deleted"], .bool(true))
    }

    func testStructuredMemoryCRUDAndAutomationDefinitionLifecycleAcrossColdReopen() throws {
        let root = try makeTempDirectory("memory-automation")
        let db = root.appendingPathComponent("memory-automation.db")
        let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: db)

            // Memory CRUD via IOSCanonicalMemoryCandidate
            let candidate = IOSCanonicalMemoryCandidate(
                record: IOSCanonicalSemanticMemoryRecord(
                    id: "ios-mem-workflow-1",
                    owner: .localProfile,
                    namespace: IOSCanonicalMemoryNamespace(kind: "personal", name: nil),
                    scope: "global",
                    kind: "preference",
                    content: "Prefer concise Swift test diagnostics with exact line context.",
                    summary: "Concise test diagnostics",
                    tags: ["swift", "testing"],
                    confidence: 0.95,
                    importance: 0.9,
                    sensitivity: nil,
                    retention: IOSCanonicalMemoryRetention(mode: "permanent", expiresAt: nil),
                    supersedes: [],
                    provenance: IOSCanonicalMemoryProvenance(
                        kind: "explicit_user",
                        source: "ios://settings/memory",
                        sessionID: nil,
                        turnID: nil,
                        itemIDs: [],
                        observedAt: "2026-11-28T00:00:00Z"
                    )
                ),
                quality: nil,
                proposedAtMs: nowMs
            )
            let savedMem = try runtime.upsertMemory(candidate)
            XCTAssertEqual(savedMem.id, "ios-mem-workflow-1")
            XCTAssertTrue(try runtime.listMemories().contains(where: { $0.id == "ios-mem-workflow-1" }))

            // Automation definition + run CRUD
            let execSpec = IOSAutomationExecutionSpec(
                prompt: "Check upcoming calendar events for tomorrow",
                executionScope: .deviceLocal(hostID: "ios-device-qa"),
                permissionProfile: IOSPermissionMode.cautious.rawValue,
                modelSelection: "use_current_default_at_admission",
                contextRefs: [.init(kind: "personal_data", id: "calendar")],
                skillRefs: []
            )
            let def = IOSCanonicalAutomationDefinition(
                id: "auto-ios-evening-prep",
                name: "Evening Prep",
                scheduleFormat: "nola.ios.scheduled_action.v1",
                enabled: true,
                revision: 1,
                executionSpec: .agent(execSpec),
                metadataJSON: #"{"hour":20,"minute":0}"#,
                createdAtMs: nowMs,
                updatedAtMs: nowMs,
                deletedAtMs: nil
            )
            let savedDef = try runtime.upsertAutomationDefinition(def)
            XCTAssertEqual(savedDef.id, "auto-ios-evening-prep")

            _ = try runtime.enqueueAutomationRun(IOSCanonicalAutomationRun(
                id: "run-evening-1",
                automationID: "auto-ios-evening-prep",
                definitionRevision: savedDef.revision,
                occurrenceKey: "2026-11-28T20:00:00Z",
                trigger: "schedule",
                status: "queued",
                attempt: 0,
                workerID: nil,
                leaseToken: nil,
                leaseUntilMs: nil,
                scheduledForMs: nowMs + 60_000,
                startedAtMs: nil,
                finishedAtMs: nil,
                nextRetryAtMs: nil,
                resultJSON: nil,
                errorJSON: nil,
                createdAtMs: nowMs,
                updatedAtMs: nowMs
            ))
        }

        // Cold reopen verifies both memory and automation persisted, then delete them
        let reopened = try IOSApplicationRuntime(databaseURL: db)
        XCTAssertTrue(try reopened.listMemories().contains(where: { $0.id == "ios-mem-workflow-1" }))
        XCTAssertTrue(try reopened.deleteMemory(id: "ios-mem-workflow-1", deletedAtMs: nowMs + 1_000))
        XCTAssertFalse(try reopened.listMemories().contains(where: { $0.id == "ios-mem-workflow-1" }))

        let defs = try reopened.listAutomationDefinitions()
        XCTAssertTrue(defs.contains(where: { $0.id == "auto-ios-evening-prep" && $0.name == "Evening Prep" }))
        let loadedRun = try XCTUnwrap(try reopened.loadAutomationRun(id: "run-evening-1"))
        XCTAssertEqual(loadedRun.automationID, "auto-ios-evening-prep")
        XCTAssertEqual(loadedRun.status, "queued")

        XCTAssertTrue(try reopened.deleteAutomationDefinition(id: "auto-ios-evening-prep", deletedAtMs: nowMs + 2_000))
        XCTAssertFalse(try reopened.listAutomationDefinitions().contains(where: { $0.id == "auto-ios-evening-prep" }))
    }

    func testDataSourceDraftOverrideAndUnsupportedV1FutureCorruptDBRejection() throws {
        let root = try makeTempDirectory("ds-override-and-reject")
        let db = root.appendingPathComponent("valid-v2.db")
        let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)

        // Draft conversation data source override without persisted session
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: db)
            let draftConvID = "ios-unsaved-draft-conv"
            let sourceID = "ios-ds-notes"
            XCTAssertEqual(try runtime.dataSourceOverride(conversationID: draftConvID, sourceID: sourceID), .inherit)
            XCTAssertTrue(try runtime.setDataSourceOverride(conversationID: draftConvID, sourceID: sourceID, value: .enabled, updatedAtMs: nowMs))
            XCTAssertEqual(try runtime.dataSourceOverride(conversationID: draftConvID, sourceID: sourceID), .enabled)
            XCTAssertTrue(try runtime.setDataSourceOverride(conversationID: draftConvID, sourceID: sourceID, value: .disabled, updatedAtMs: nowMs + 50))
        }
        let reopened = try IOSApplicationRuntime(databaseURL: db)
        XCTAssertEqual(
            try reopened.dataSourceOverride(conversationID: "ios-unsaved-draft-conv", sourceID: "ios-ds-notes"),
            .disabled
        )

        // Legacy v1 DB (user_version = 1) must be rejected without modifying a single byte
        let v1DB = root.appendingPathComponent("legacy-v1-workflow.db")
        var conn: OpaquePointer?
        XCTAssertEqual(sqlite3_open(v1DB.path, &conn), SQLITE_OK)
        XCTAssertEqual(
            sqlite3_exec(conn, "CREATE TABLE legacy_notes (body TEXT); INSERT INTO legacy_notes VALUES ('v1-intact'); PRAGMA user_version = 1;", nil, nil, nil),
            SQLITE_OK
        )
        sqlite3_close(conn)
        let v1Bytes = try Data(contentsOf: v1DB)
        XCTAssertThrowsError(try IOSApplicationRuntime(databaseURL: v1DB))
        XCTAssertEqual(try Data(contentsOf: v1DB), v1Bytes)

        // Unknown future schema version (user_version = 99) must also be rejected without modifying bytes
        let futureDB = root.appendingPathComponent("future-v99-workflow.db")
        var fConn: OpaquePointer?
        XCTAssertEqual(sqlite3_open(futureDB.path, &fConn), SQLITE_OK)
        XCTAssertEqual(
            sqlite3_exec(fConn, "CREATE TABLE future_notes (body TEXT); INSERT INTO future_notes VALUES ('v99-intact'); PRAGMA user_version = 99;", nil, nil, nil),
            SQLITE_OK
        )
        sqlite3_close(fConn)
        let futureBytes = try Data(contentsOf: futureDB)
        XCTAssertThrowsError(try IOSApplicationRuntime(databaseURL: futureDB))
        XCTAssertEqual(try Data(contentsOf: futureDB), futureBytes)
    }
}
