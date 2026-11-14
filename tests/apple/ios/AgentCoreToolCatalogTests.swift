// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class AgentCoreToolCatalogTests: XCTestCase {
    private func string(
        _ key: String,
        in value: AgentCoreJSONValue
    ) -> String? {
        guard case .object(let object) = value,
              case .string(let result)? = object[key] else { return nil }
        return result
    }

    func testDeviceCoordinatorProjectsGrantedAndScopedPermissionRules() {
        let tool = AgentCoreResolvedTool(
            toolId: "device_search_calendar_events",
            name: "Search calendar",
            enabled: true,
            route: "device",
            capability: "calendar",
            permission: "system_permission",
            authorization: "granted",
            risk: "read_only",
            requiresResultEgress: false,
            reason: "",
            schema: .object([:])
        )
        let rules = DeviceAgentCoordinator.permissionRules(
            enabledTools: [tool],
            scopedGrants: [
                .init(
                    capability: "files_read",
                    route: "device",
                    scope: "file:///tmp/reference.txt",
                    expiresAtMs: Int64.max
                ),
            ],
            permissionMode: .cautious
        )

        guard case .object(let root) = rules,
              case .array(let values)? = root["rules"] else {
            return XCTFail("Expected Core permission rules")
        }
        XCTAssertEqual(values.count, 3)
        XCTAssertEqual(string("source", in: values[0]), "product_default")
        XCTAssertEqual(string("capability_pattern", in: values[1]), "calendar")
        XCTAssertEqual(string("resource_pattern", in: values[2]), "file:///tmp/reference.txt")
    }

    func testDeviceCoordinatorProjectsPermissionProfileBaseline() throws {
        func firstRule(mode: IOSPermissionMode, automated: Bool = false) -> AgentCoreJSONValue? {
            let value = DeviceAgentCoordinator.permissionRules(
                enabledTools: [], scopedGrants: [], permissionMode: mode, isAutomated: automated
            )
            guard case .object(let root) = value,
                  case .array(let rules)? = root["rules"] else { return nil }
            return rules.first
        }

        XCTAssertEqual(string("source", in: try XCTUnwrap(firstRule(mode: .cautious))), "product_default")
        XCTAssertEqual(string("source", in: try XCTUnwrap(firstRule(mode: .allowAll))), "invocation_policy")
        XCTAssertEqual(
            string("source", in: try XCTUnwrap(firstRule(mode: .allowAll, automated: true))),
            "invocation_policy"
        )
    }

    func testNativeCatalogDeclaresSemanticRiskWithoutGuessingOperationNames() throws {
        let expected = ["skill_invoke": "ordinary_read", "memory_search": "ordinary_read",
            "device_read_file": "ordinary_read", "device_write_file": "unknown",
            "device_clipboard_read": "sensitive_read", "device_clipboard_write": "overwrite",
            "device_get_location": "sensitive_read", "device_search_calendar_events": "sensitive_read",
            "device_create_calendar_event": "new_file"]
        var actual: [String: String] = [:]
        for schema in AgentCoreToolCatalog.deviceTools {
            guard case .object(let root) = schema, case .object(let function)? = root["function"],
                  case .string(let name)? = function["name"], case .object(let metadata)? = root["x_nola"],
                  case .object(let contract)? = metadata["permission_contract"],
                  case .string(let risk)? = contract["risk"] else {
                return XCTFail("Every native catalog entry must declare its trusted semantic risk")
            }
            actual[name] = risk
        }
        XCTAssertEqual(actual, expected)
    }

    func testAllowAllDoesNotPretendPendingSystemAuthorizationWasGranted() throws {
        let tool = AgentCoreResolvedTool(toolId: "device_get_location", name: "Get location", enabled: true,
            route: "device", capability: "location", permission: "system_permission", authorization: "ask",
            risk: "read_only", requiresResultEgress: true, reason: "", schema: .object([:]))
        let result = DeviceAgentCoordinator.permissionRules(enabledTools: [tool], scopedGrants: [], permissionMode: .allowAll)
        guard case .object(let root) = result, case .array(let rules)? = root["rules"] else {
            return XCTFail("Expected the formal invocation and platform rules")
        }
        XCTAssertEqual(rules.count, 2)
        XCTAssertEqual(string("source", in: rules[0]), "invocation_policy")
        XCTAssertEqual(string("source", in: rules[1]), "host")
        XCTAssertEqual(string("decision", in: rules[1]), "ask")
    }

    func testDeviceCoordinatorEchoesEffectDeliveryLeaseInHostResponse() throws {
        let delivery: AgentCoreJSONValue = .object([
            "attempt_id": .string("attempt-1"),
            "worker_generation": .string("generation-1"),
            "lease_owner": .string("ios-device"),
            "lease_token": .string("lease-token"),
            "lease_expires_at_ms": .number(9_999_999),
        ])
        let response = try DeviceAgentCoordinator.hostResponseEnvelope(
            request: .object([
                "request_id": .string("request-1"),
                "effect_id": .string("effect-1"),
                "attempt_id": .string("attempt-1"),
                "delegated_invocation_id": .string("delegated-1"),
                "delivery": delivery,
            ]),
            payload: .object(["type": .string("completed")]),
            sequence: 1,
            attemptID: "attempt-2"
        )
        guard case .object(let envelope) = response else {
            return XCTFail("Expected a Host response envelope")
        }
        XCTAssertEqual(envelope["delivery"], delivery)
        XCTAssertEqual(envelope["attempt_id"], .string("attempt-2"))
        XCTAssertEqual(envelope["delegated_invocation_id"], .string("delegated-1"))
    }

    func testIOSFileResultsExposeCoreArtifacts() {
        let artifacts = iosAgentFileArtifacts(
            output: .object(["uri": .string("file:///tmp/report.docx")]),
            capability: .filesWrite
        )

        XCTAssertEqual(artifacts.count, 1)
        XCTAssertEqual(string("name", in: artifacts[0]), "report.docx")
        XCTAssertEqual(string("operation", in: artifacts[0]), "create")
        XCTAssertEqual(string("uri", in: artifacts[0]), "file:///tmp/report.docx")
        XCTAssertTrue(iosAgentFileArtifacts(
            output: .object(["uri": .string("/tmp/legacy-report.docx")]),
            capability: .filesWrite
        ).isEmpty)
    }

    func testCanonicalConversationDiscoverySurvivesColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-conversation-discovery-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let first = try IOSApplicationRuntime(databaseURL: database)
        try first.createConversation(
            id: "conversation/with unsafe path",
            title: "Canonical conversation",
            settingsJSON: nil,
            createdAtMs: 100,
            updatedAtMs: 100
        )

        let reopened = try IOSApplicationRuntime(databaseURL: database)
        XCTAssertEqual(try reopened.listConversations().map(\.id), ["conversation/with unsafe path"])
        XCTAssertNotNil(try reopened.loadConversation(id: "conversation/with unsafe path"))
    }

    func testCanonicalConversationDiscoveryIgnoresUnrelatedLegacyFiles() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-sidecar-isolation-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let store = try IOSApplicationRuntime(databaseURL: database)
        try store.createConversation(
            id: "conversation-a",
            title: "A",
            settingsJSON: nil,
            createdAtMs: 100,
            updatedAtMs: 100
        )
        try Data("not-json".utf8).write(to: root.appendingPathComponent("corrupt.config.json"))

        XCTAssertEqual(
            try IOSApplicationRuntime(databaseURL: database).listConversations().map(\.id),
            ["conversation-a"]
        )
    }

    func testSharedToolPolicyABIIsAvailable() {
        XCTAssertEqual(AgentCoreToolPolicyResolver.toolPolicyABIVersion, 1)
    }

    func testSharedGoldenFixtureResolvesThroughProductionABI() throws {
        let fixtureURL = try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "tool_policy_v1_golden",
                withExtension: "json"
            )
        )
        let fixtureData = try Data(contentsOf: fixtureURL)
        let root = try XCTUnwrap(JSONSerialization.jsonObject(with: fixtureData) as? [String: Any])
        let input = try XCTUnwrap(root["input"] as? [String: Any])
        let expected = try XCTUnwrap(root["expected"] as? [String: Any])

        let output = try AgentCoreToolPolicyResolver.resolveToolPolicyJSON(
            JSONSerialization.data(withJSONObject: input, options: [.sortedKeys])
        )

        XCTAssertEqual(output.schemaVersion, 1)
        XCTAssertEqual(output.enabledTools.map(\.toolId), expected["enabled_tool_ids"] as? [String])
        let expectedDisabled = try XCTUnwrap(expected["disabled_tools"] as? [[String: Any]])
        XCTAssertEqual(output.disabledTools.map(\.toolId), expectedDisabled.compactMap { $0["tool_id"] as? String })
        XCTAssertEqual(output.disabledTools.map(\.reason), expectedDisabled.compactMap { $0["reason"] as? String })

        let receipt = try XCTUnwrap(expected["receipt"] as? [String: Any])
        let expectedTool = try XCTUnwrap(receipt["enabled_tool"] as? [String: Any])
        let resolved = try XCTUnwrap(output.enabledTools.first)
        XCTAssertEqual(resolved.route, expectedTool["route"] as? String)
        XCTAssertEqual(resolved.capability, expectedTool["capability"] as? String)
        XCTAssertEqual(resolved.permission, expectedTool["permission"] as? String)
        XCTAssertEqual(resolved.authorization, expectedTool["authorization"] as? String)
        XCTAssertEqual(resolved.risk, expectedTool["risk"] as? String)
        XCTAssertEqual(resolved.requiresResultEgress, expectedTool["requires_result_egress"] as? Bool)
        XCTAssertEqual(output.manifestValidUntilMs, (receipt["manifest_valid_until_ms"] as? NSNumber)?.int64Value)
        let digestLength = (receipt["digest_hex_length"] as? NSNumber)?.intValue
        XCTAssertEqual(output.policyDigest.count, digestLength)
        XCTAssertEqual(output.manifestDigest.count, digestLength)
    }

    func testHostModeDoesNotInjectDeviceTools() {
        XCTAssertTrue(AgentCoreToolCatalog.tools(for: .hostRemote).isEmpty)
    }

    func testLocalCatalogComesFromCorePlan() throws {
        let plan = try AgentCoreToolCatalog.resolvePlan(
            for: .deviceLocal,
            isForeground: true
        )
        let resolved = plan.enabledTools + plan.disabledTools
        XCTAssertFalse(resolved.isEmpty)
        XCTAssertTrue(resolved.allSatisfy { $0.route == "device" })
        XCTAssertTrue(resolved.contains { $0.name == "device_read_file" })
        XCTAssertTrue(resolved.contains { $0.name == "device_get_location" })
    }

    func testCatalogAndExecutorShareOnlyCanonicalDeviceToolIDs() throws {
        let expected: Set<String> = [
            "device_read_file",
            "device_write_file",
            "device_clipboard_read",
            "device_clipboard_write",
            "device_get_location",
            "device_search_calendar_events",
            "device_create_calendar_event",
            "memory_search",
            "skill_invoke",
            "render_ui",
            "delegate_expert",
        ]
        XCTAssertEqual(Set(IOSAgentHostExecutor.installedToolCapabilities.keys), expected)

        let plan = try AgentCoreToolCatalog.resolvePlan(
            for: .deviceLocal,
            isForeground: true
        )
        XCTAssertEqual(Set((plan.enabledTools + plan.disabledTools).map(\.toolId)), expected.subtracting(["render_ui", "delegate_expert"]))
        XCTAssertTrue(expected.isDisjoint(with: [
            "device_read_clipboard",
            "device_write_clipboard",
            "device_list_calendar_events",
        ]))
    }

    @MainActor
    func testCoreUISurfaceUsesBoundPermissionPlanAndHonorsToolDenial() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ui-plan-\(UUID())")
        let runtime = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("state.db"))
        let previous = IOSApplicationRuntime.shared
        IOSApplicationRuntime.installShared(runtime)
        defer { IOSApplicationRuntime.installShared(previous); try? FileManager.default.removeItem(at: directory) }
        let identity = AgentCoreExpectedIdentity(hostId: "ios-local", clientId: "test", sessionId: "ui-plan")
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal, expectedIdentity: identity, isForeground: true)
        let tool = try XCTUnwrap(plan.resolvedTool(id: "render_ui", route: "device",
            hostId: identity.hostId, clientId: identity.clientId, sessionId: identity.sessionId))
        XCTAssertEqual(tool.capability, "none")
        XCTAssertNotNil(IOSAgentPermissionRequest(requestId: "request", allowConversation: false, callId: "call",
            resolvedTool: tool, plan: plan, rationale: "Publish the requested form"))
        let denied = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal, expectedIdentity: identity,
            isForeground: true, deniedTools: ["render_ui"])
        XCTAssertNil(denied.resolvedTool(id: "render_ui", route: "device",
            hostId: identity.hostId, clientId: identity.clientId, sessionId: identity.sessionId))
    }

    @MainActor
    func testDeviceCoordinatorForwardsCoreProjectedToolsWithNativeProviderCapability() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let identity = AgentCoreExpectedIdentity(
            hostId: "ios-local",
            clientId: Bundle.main.bundleIdentifier ?? "nola-ios",
            sessionId: "native-tool-contract"
        )
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(
            for: .deviceLocal,
            expectedIdentity: identity,
            isForeground: true,
            scopedGrants: []
        )
        XCTAssertTrue(
            plan.output.enabledTools.contains { $0.toolId == "skill_invoke" },
            "skill_invoke must survive Core policy resolution; disabled=\(plan.output.disabledTools)"
        )
        XCTAssertTrue(plan.schemas.contains { tool in
            guard case .object(let root) = tool,
                  case .object(let function)? = root["function"],
                  case .string(let name)? = function["name"] else { return false }
            return name == "skill_invoke"
        }, "Prepared schemas must include skill_invoke; enabled=\(plan.output.enabledTools), disabled=\(plan.output.disabledTools)")
        let model = CoreRequestCapturingModelClient()
        let hostExecutor = IOSAgentHostExecutor()
        let applicationStore = try IOSApplicationRuntime(
            databaseURL: directory.appendingPathComponent("application.db")
        )
        let turnIdentity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let canonicalApplication = IOSCanonicalAgentApplication(
            store: applicationStore,
            descriptor: .init(
                conversationID: "native-tool-contract",
                conversationTitle: "Native tool contract",
                identity: turnIdentity,
                userText: "Use an enabled Skill if one is available.",
                userContentJSON: nil,
                contextCheckpointID: nil
            )
        )
        await hostExecutor.acquireAgentSession(sessionId: "native-tool-contract")
        defer {
            Task { await hostExecutor.releaseAgentSession(sessionId: "native-tool-contract") }
        }
        let coordinator = DeviceAgentCoordinator(
            modelClient: model,
            hostExecutor: hostExecutor,
            toolPlan: plan,
            canonicalApplication: canonicalApplication,
            preparedHostInvocation: try makeIOSDeterministicHostInvocation(store: applicationStore, model: "provider-model",
                outputTokens: 1_024, capabilities: model.coreModelCapabilityProfile)
        )

        let stream = await coordinator.start(
            sessionId: "native-tool-contract",
            input: "Use an enabled Skill if one is available.",
            history: [],
            contextComponents: [],
            turnIdentity: turnIdentity,
            config: .init(
                platform: .ios,
                executionMode: .deviceLocal,
                model: "provider-model",
                maxIterations: 1,
                contextWindowTokens: 64_000,
                reservedOutputTokens: 1_024
            )
        )
        for try await _ in stream {}

        let request = try XCTUnwrap(model.request)
        XCTAssertTrue(request.invocationPlan.nativeToolCalls)
        XCTAssertTrue(request.invocationPlan.streamToolCalls)
        XCTAssertTrue(request.tools.contains { tool in
            guard case .object(let root) = tool,
                  case .object(let function)? = root["function"],
                  case .string(let name)? = function["name"] else { return false }
            return name == "skill_invoke"
        }, "Core must expose skill_invoke in the final model request: \(request.tools)")
        let skillTool = try XCTUnwrap(request.tools.first { tool in
            guard case .object(let root) = tool,
                  case .object(let function)? = root["function"],
                  case .string(let name)? = function["name"] else { return false }
            return name == "skill_invoke"
        })
        guard case .object(let root) = skillTool,
              case .object(let function)? = root["function"],
              case .object(let parameters)? = function["parameters"] else {
            return XCTFail("skill_invoke parameters must be an object schema: \(skillTool)")
        }
        XCTAssertEqual(parameters["type"], .string("object"))
        XCTAssertNil(parameters["function"], "A complete tool definition must never be nested under function.parameters")
    }

    func testForgedRuntimeMetadataCannotOverrideCoreCapabilityOrRisk() throws {
        let nowMs: Int64 = 1_000_000
        let identity = AgentCoreExpectedIdentity(
            hostId: "ios-test",
            clientId: "ios-client",
            sessionId: "ios-session"
        )
        let manifestId = "ios-test:forged-runtime-metadata"
        let forgedSchema: AgentCoreJSONValue = .object([
            "type": .string("function"),
            "function": .object([
                "name": .string("device_read_file"),
                "parameters": .object(["type": .string("object")]),
            ]),
            "x_nola": .object([
                "capability": .string("location"),
                "risk": .string("critical"),
                "permission": .string("none"),
            ]),
        ])
        let output = try AgentCoreToolPolicyResolver.resolveToolPolicy(
            AgentCoreToolPolicyInput(
                platform: "ios",
                nowMs: nowMs,
                expectedIdentity: identity,
                runtime: .init(
                    manifestId: manifestId,
                    manifestVersion: 2,
                    issuedAtMs: nowMs - 1_000,
                    expiresAtMs: nowMs + 60_000,
                    hostId: "ios-test",
                    clientId: "ios-client",
                    sessionId: "ios-session",
                    routes: ["device"],
                    capabilities: ["files_read", "location"],
                    authorizations: ["files_read": "granted", "location": "granted"],
                    resultEgressReceipts: [
                        .init(
                            toolId: "device_read_file",
                            route: "device",
                            manifestId: manifestId,
                            hostId: "ios-test",
                            clientId: "ios-client",
                            sessionId: "ios-session",
                            issuedAtMs: nowMs - 1_000,
                            expiresAtMs: nowMs + 50_000,
                            allowResultEgress: true
                        ),
                    ],
                    tools: [
                        .init(
                            toolId: "device_read_file",
                            handlerId: "device_read_file",
                            schema: forgedSchema,
                            route: "device"
                        ),
                    ]
                ),
                userPolicy: .init(maxRisk: "critical", allowResultEgressRoutes: ["device"]),
                scopedGrants: [
                    .init(capability: "files_read", route: "device", scope: "fixture", expiresAtMs: nowMs + 50_000),
                ],
                resultEgressGrants: [
                    .init(
                        toolId: "device_read_file",
                        route: "device",
                        manifestId: manifestId,
                        hostId: "ios-test",
                        clientId: "ios-client",
                        sessionId: "ios-session",
                        expiresAtMs: nowMs + 50_000
                    ),
                ]
            )
        )

        let resolved = try XCTUnwrap(output.enabledTools.first)
        XCTAssertEqual(resolved.capability, "files_read")
        XCTAssertEqual(resolved.risk, "read_only")
        XCTAssertEqual(resolved.permission, "scoped_resource")

        let exposedSchema = AgentCoreToolCatalog.schema(for: resolved, resultEgressAllowed: true)
        guard case .object(let root) = exposedSchema,
              case .object(let metadata)? = root["x_nola"] else {
            XCTFail("Expected sanitized Core metadata in exposed schema")
            return
        }
        XCTAssertEqual(metadata["capability"], .string("files_read"))
        XCTAssertEqual(metadata["risk"], .string("read_only"))
        XCTAssertEqual(metadata["permission"], .string("scoped_resource"))
        XCTAssertEqual(metadata["authorization"], .string("granted"))
        XCTAssertEqual(metadata["target"], .string("device"))
    }

    func testUserCapabilityDenyIsAppliedByCore() throws {
        let plan = try AgentCoreToolCatalog.resolvePlan(
            for: .deviceLocal,
            isForeground: true,
            deniedCapabilities: ["location"]
        )
        let location = try XCTUnwrap(plan.disabledTools.first { $0.name == "device_get_location" })
        XCTAssertEqual(location.reason, "capability_denied_by_user")
    }

    func testForegroundOnlyHandlersAreUnavailableInBackground() throws {
        let plan = try AgentCoreToolCatalog.resolvePlan(
            for: .deviceLocal,
            isForeground: false
        )
        for name in ["device_clipboard_read", "device_get_location", "device_search_calendar_events"] {
            let tool = try XCTUnwrap(plan.disabledTools.first { $0.name == name })
            XCTAssertTrue(["runtime_capability_unavailable", "authorization_unavailable"].contains(tool.reason))
        }
    }

    func testHybridToolsFailClosedWithoutResultEgressConsent() throws {
        let plan = try AgentCoreToolCatalog.resolvePlan(
            for: .hybrid,
            isForeground: true
        )
        XCTAssertTrue(plan.enabledTools.allSatisfy { !$0.requiresResultEgress })
        XCTAssertTrue(plan.disabledTools.contains { $0.requiresResultEgress })
        XCTAssertTrue(plan.disabledTools.contains { $0.reason == "result_egress_receipt_missing" })
    }

    func testDeviceResultEgressRequiresValidReceiptRouteAndSessionGrant() throws {
        let output = try AgentCoreToolPolicyResolver.resolveToolPolicy(makeResultEgressInput())
        XCTAssertEqual(output.enabledTools.map(\.toolId), ["device_read_file"])
    }

    func testDeviceResultEgressFailsClosedWhenAnyProofLayerIsMissing() throws {
        let cases: [(EgressMutation, String)] = [
            (.missingReceipt, "result_egress_receipt_missing"),
            (.missingAllowedRoute, "result_egress_route_not_allowed"),
            (.missingGrant, "result_egress_session_grant_missing"),
        ]
        for (mutation, expectedReason) in cases {
            let output = try AgentCoreToolPolicyResolver.resolveToolPolicy(makeResultEgressInput(mutation))
            XCTAssertTrue(output.enabledTools.isEmpty, "Mutation \(mutation) must fail closed")
            XCTAssertEqual(output.disabledTools.first?.reason, expectedReason)
        }
    }

    func testDeviceResultEgressFailsClosedOnIdentitySessionOrExpiryMismatch() throws {
        let cases: [(EgressMutation, String)] = [
            (.receiptIdentityMismatch, "result_egress_receipt_missing"),
            (.receiptSessionMismatch, "result_egress_receipt_missing"),
            (.grantSessionMismatch, "result_egress_session_grant_missing"),
            (.expiredReceipt, "result_egress_receipt_missing"),
            (.grantExceedsReceiptExpiry, "result_egress_session_grant_missing"),
        ]
        for (mutation, expectedReason) in cases {
            let output = try AgentCoreToolPolicyResolver.resolveToolPolicy(makeResultEgressInput(mutation))
            XCTAssertTrue(output.enabledTools.isEmpty, "Mutation \(mutation) must fail closed")
            XCTAssertEqual(output.disabledTools.first?.reason, expectedReason)
        }
    }

    func testDeviceResultEgressRequiredReceiptFieldsFailClosedWhenOmitted() throws {
        let requiredFields = [
            "tool_id", "route", "manifest_id", "host_id", "client_id", "session_id",
            "issued_at_ms", "expires_at_ms", "allow_result_egress",
        ]
        let encoded = try JSONEncoder().encode(makeResultEgressInput())
        let root = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        for field in requiredFields {
            var candidate = root
            var runtime = try XCTUnwrap(candidate["runtime"] as? [String: Any])
            var receipts = try XCTUnwrap(runtime["result_egress_receipts"] as? [[String: Any]])
            receipts[0].removeValue(forKey: field)
            runtime["result_egress_receipts"] = receipts
            candidate["runtime"] = runtime
            let data = try JSONSerialization.data(withJSONObject: candidate, options: [.sortedKeys])
            do {
                let output = try AgentCoreToolPolicyResolver.resolveToolPolicyJSON(data)
                XCTAssertTrue(output.enabledTools.isEmpty, "Missing \(field) must fail closed")
            } catch {
                // Rejecting malformed input during decoding is also fail-closed.
            }
        }
    }

    private enum EgressMutation {
        case none
        case missingReceipt
        case missingAllowedRoute
        case missingGrant
        case receiptIdentityMismatch
        case receiptSessionMismatch
        case grantSessionMismatch
        case expiredReceipt
        case grantExceedsReceiptExpiry
    }

    private func makeResultEgressInput(_ mutation: EgressMutation = .none) -> AgentCoreToolPolicyInput {
        let nowMs: Int64 = 1_000_000
        let manifestId = "ios-egress-fixture"
        let hostId = "ios-device-1"
        let clientId = "ios-controller-1"
        let sessionId = "conversation-1"
        let receiptExpiresAt = mutation == .expiredReceipt ? nowMs : nowMs + 50_000
        let grantExpiresAt = mutation == .grantExceedsReceiptExpiry ? nowMs + 55_000 : nowMs + 50_000
        let receipts: [AgentCoreRuntimeResultEgressReceipt] = mutation == .missingReceipt ? [] : [
            .init(
                toolId: "device_read_file",
                route: "device",
                manifestId: manifestId,
                hostId: mutation == .receiptIdentityMismatch ? "other-host" : hostId,
                clientId: clientId,
                sessionId: mutation == .receiptSessionMismatch ? "other-session" : sessionId,
                issuedAtMs: nowMs - 1_000,
                expiresAtMs: receiptExpiresAt,
                allowResultEgress: true
            ),
        ]
        let grants: [AgentCoreResultEgressGrant] = mutation == .missingGrant ? [] : [
            .init(
                toolId: "device_read_file",
                route: "device",
                manifestId: manifestId,
                hostId: hostId,
                clientId: clientId,
                sessionId: mutation == .grantSessionMismatch ? "other-session" : sessionId,
                expiresAtMs: grantExpiresAt
            ),
        ]
        return AgentCoreToolPolicyInput(
            platform: "ios",
            nowMs: nowMs,
            expectedIdentity: .init(hostId: hostId, clientId: clientId, sessionId: sessionId),
            runtime: .init(
                manifestId: manifestId,
                manifestVersion: 2,
                issuedAtMs: nowMs - 1_000,
                expiresAtMs: nowMs + 60_000,
                hostId: hostId,
                clientId: clientId,
                sessionId: sessionId,
                routes: ["device"],
                capabilities: ["files_read"],
                authorizations: ["files_read": "granted"],
                resultEgressReceipts: receipts,
                tools: [
                    .init(
                        toolId: "device_read_file",
                        handlerId: "device_read_file",
                        schema: .object([
                            "type": .string("function"),
                            "function": .object([
                                "name": .string("device_read_file"),
                                "parameters": .object(["type": .string("object")]),
                            ]),
                        ]),
                        route: "device"
                    ),
                ]
            ),
            userPolicy: .init(
                allowResultEgressRoutes: mutation == .missingAllowedRoute ? [] : ["device"]
            ),
            scopedGrants: [
                .init(capability: "files_read", route: "device", scope: "fixture", expiresAtMs: nowMs + 50_000),
            ],
            resultEgressGrants: grants
        )
    }
}

private final class CoreRequestCapturingModelClient: AgentModelClient, @unchecked Sendable {
    var request: AgentModelRequest?
    nonisolated let coreModelCapabilityProfile: AgentCoreJSONValue? = .object([
        "profile_id": .string("ios-native-event-test-v1"),
        "revision": .number(1),
        "instruction_profile": .string("generic"),
        "native_tool_calls": .bool(true),
        "parallel_tool_calls": .bool(false),
        "reasoning": .bool(false),
        "reasoning_continuity": .bool(false),
        "prompt_cache": .bool(false),
        "context_window_tokens": .null,
        "max_output_tokens": .null,
        "request_continuation": .bool(true),
        "max_request_continuations": .number(2),
        "stream_text": .bool(true),
        "stream_reasoning": .bool(false),
        "stream_tool_calls": .bool(true),
    ])

    func generateAgentTurn(
        request: AgentModelRequest
    ) -> AsyncThrowingStream<AgentModelEvent, Error> {
        self.request = request
        return AsyncThrowingStream { continuation in
            continuation.yield(.completed(.init(content: "done", finishReason: "stop")))
            continuation.finish()
        }
    }
}
