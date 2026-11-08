import XCTest
import ApplicationServices
@testable import Nola

@MainActor
final class MacPersonalIntelligenceCoreTests: XCTestCase {
    func testPermissionRefreshPolicyPollsUnresolvedForegroundPermissionsPromptly() {
        XCTAssertEqual(
            MacCapabilityPermissionRefreshPolicy.intervalSeconds(
                applicationIsActive: true,
                hasUnresolvedSystemPermission: true
            ),
            2
        )
        XCTAssertEqual(
            MacCapabilityPermissionRefreshPolicy.intervalSeconds(
                applicationIsActive: true,
                hasUnresolvedSystemPermission: false
            ),
            10
        )
        XCTAssertEqual(
            MacCapabilityPermissionRefreshPolicy.intervalSeconds(
                applicationIsActive: false,
                hasUnresolvedSystemPermission: true
            ),
            30
        )
    }

    func testAuthorizedAccessibilityDoesNotBecomePermissionRequiredWhenTargetIsTemporarilyUnreadable() {
        let authorized = MacAccessibilityPermissionSnapshot(
            systemTrusted: true,
            systemWideReadable: false,
            treeReadable: false
        )
        let unauthorized = MacAccessibilityPermissionSnapshot(
            systemTrusted: false,
            systemWideReadable: false,
            treeReadable: false
        )

        XCTAssertEqual(authorized.targetState(isReadable: false), .targetUnavailable)
        XCTAssertEqual(unauthorized.targetState(isReadable: false), .permissionRequired)
        XCTAssertEqual(authorized.targetState(isReadable: true), .readable)
    }

    func testObservedRuntimeCapabilityOverridesLaggingSystemPreflight() {
        let accessibility = MacAccessibilityPermissionSnapshot(
            systemTrusted: false,
            systemWideReadable: false,
            treeReadable: false,
            runtimeObserved: true
        )
        let screenRecording = MacScreenRecordingPermissionSnapshot(
            systemPreflight: false,
            runtimeCaptureSucceeded: true
        )

        XCTAssertTrue(accessibility.isAuthorized)
        XCTAssertEqual(accessibility.targetState(isReadable: false), .targetUnavailable)
        XCTAssertTrue(screenRecording.isAuthorized)
    }

    func testFocusedExternalElementOverridesLaggingAccessibilityPreflight() {
        let accessibility = MacAccessibilityPermissionSnapshot(
            systemTrusted: false,
            systemWideReadable: false,
            focusedElementReadable: true,
            treeReadable: false
        )

        XCTAssertTrue(accessibility.isAuthorized)
        XCTAssertEqual(accessibility.targetState(isReadable: false), .targetUnavailable)
    }

    func testEventPostingPreflightRecognizesGrantedAccessibilityWhenAXTrustLags() {
        let accessibility = MacAccessibilityPermissionSnapshot(
            systemTrusted: false,
            eventPostingAuthorized: true,
            systemWideReadable: false,
            focusedElementReadable: false,
            treeReadable: false
        )

        XCTAssertTrue(accessibility.isAuthorized)
        XCTAssertEqual(accessibility.targetState(isReadable: false), .targetUnavailable)
    }

    func testAccessibilitySnapshotNeverTreatsAnAuthorizedButUnavailableTargetAsMissingPermission() {
        let accessibility = MacAccessibilityPermissionSnapshot(
            systemTrusted: false,
            systemWideReadable: false,
            focusedElementReadable: false,
            treeReadable: true
        )

        XCTAssertTrue(accessibility.isAuthorized)
        XCTAssertEqual(accessibility.targetState(isReadable: false), .targetUnavailable)
    }

    func testAccessibilityAppMatchingRejectsHelpersAndPrefersExactAppName() {
        XCTAssertNil(
            MacAccessibilityApplicationResolver.applicationMatchScore(
                requested: "chrome",
                localizedName: "Google Chrome Helper (Renderer)",
                bundleIdentifier: "com.google.Chrome.helper.renderer",
                isRegularApplication: false,
                isNola: false
            )
        )
        XCTAssertGreaterThan(
            MacAccessibilityApplicationResolver.applicationMatchScore(
                requested: "google chrome",
                localizedName: "Google Chrome",
                bundleIdentifier: "com.google.Chrome",
                isRegularApplication: true,
                isNola: false
            ) ?? 0,
            MacAccessibilityApplicationResolver.applicationMatchScore(
                requested: "google chrome",
                localizedName: "Google Chrome Canary",
                bundleIdentifier: "com.google.Chrome.canary",
                isRegularApplication: true,
                isNola: false
            ) ?? 0
        )
    }

    func testAccessibilityAuthorizationUsesTheRealFocusedApplicationProbeWhenTrustIsStale() {
        XCTAssertTrue(
            MacSystemPermissionProbe.accessibilityIsEffective(
                systemTrusted: false,
                focusedApplicationResult: .success,
                hasFocusedApplication: true
            )
        )
        XCTAssertFalse(
            MacSystemPermissionProbe.accessibilityIsEffective(
                systemTrusted: false,
                focusedApplicationResult: .apiDisabled,
                hasFocusedApplication: false
            )
        )
        XCTAssertFalse(
            MacSystemPermissionProbe.accessibilityIsEffective(
                systemTrusted: false,
                focusedApplicationResult: .success,
                hasFocusedApplication: true,
                focusedTreeProbeSucceeded: false
            )
        )
        XCTAssertTrue(
            MacSystemPermissionProbe.accessibilityIsEffective(
                systemTrusted: true,
                focusedApplicationResult: .cannotComplete,
                hasFocusedApplication: false
            )
        )
        XCTAssertFalse(
            MacSystemPermissionProbe.accessibilityIsEffective(
                systemTrusted: false,
                focusedApplicationResult: .success,
                hasFocusedApplication: true,
                focusedTreeProbeSucceeded: true,
                probeTargetsExternalApplication: false
            ),
            "Reading Nola's own AX tree must not be treated as permission to control other apps"
        )
    }

    func testSystemWideAccessibilityProbeOnlyAcceptsAnExternalFocusedApplication() {
        XCTAssertTrue(
            MacSystemPermissionProbe.systemWideProbeIsEffective(
                focusedApplicationResult: .success,
                focusedPIDResult: .success,
                focusedApplicationPID: 88,
                currentPID: 7,
                probeTargetsExternalApplication: true
            )
        )
        XCTAssertFalse(
            MacSystemPermissionProbe.systemWideProbeIsEffective(
                focusedApplicationResult: .success,
                focusedPIDResult: .success,
                focusedApplicationPID: 7,
                currentPID: 7,
                probeTargetsExternalApplication: false
            ),
            "Nola's own focused AX element cannot prove cross-application permission"
        )
        XCTAssertFalse(
            MacSystemPermissionProbe.systemWideProbeIsEffective(
                focusedApplicationResult: .cannotComplete,
                focusedPIDResult: .success,
                focusedApplicationPID: 88,
                currentPID: 7,
                probeTargetsExternalApplication: true
            )
        )
    }

    func testAccessibilityTargetProbeAcceptsAttributeNamesOrAReadableValue() {
        XCTAssertTrue(
            MacSystemPermissionProbe.elementProbeIsEffective(
                attributeNamesResult: .success,
                attributeCount: 12,
                valueProbeSucceeded: false
            ),
            "A readable AX attribute catalog is direct evidence that the target application is accessible"
        )
        XCTAssertTrue(
            MacSystemPermissionProbe.elementProbeIsEffective(
                attributeNamesResult: .cannotComplete,
                attributeCount: 0,
                valueProbeSucceeded: true
            ),
            "A successful target attribute read must override a stale catalog/preflight failure"
        )
        XCTAssertFalse(
            MacSystemPermissionProbe.elementProbeIsEffective(
                attributeNamesResult: .apiDisabled,
                attributeCount: 0,
                valueProbeSucceeded: false
            )
        )
    }

    func testAccessibilityTargetPrefersTheExternalFrontmostAppThenTheTopVisibleWindow() {
        XCTAssertEqual(
            MacAccessibilityApplicationResolver.selectTargetPID(
                frontmostPID: 42,
                currentPID: 7,
                orderedWindowOwnerPIDs: [7, 88, 42],
                eligiblePIDs: [42, 88]
            ),
            42
        )
        XCTAssertEqual(
            MacAccessibilityApplicationResolver.selectTargetPID(
                frontmostPID: 7,
                currentPID: 7,
                orderedWindowOwnerPIDs: [7, 88, 42],
                eligiblePIDs: [42, 88]
            ),
            88
        )
    }

    func testAccessibilityTraversalStartsFromApplicationWindowsAndVisibleContent() {
        let attributes = Set(MacComputerUseService.accessibilityDescendantAttributeNames)

        XCTAssertTrue(attributes.contains(kAXFocusedWindowAttribute))
        XCTAssertTrue(attributes.contains(kAXWindowsAttribute))
        XCTAssertTrue(attributes.contains(kAXChildrenAttribute))
        XCTAssertTrue(attributes.contains(kAXVisibleChildrenAttribute))
        XCTAssertTrue(attributes.contains(kAXContentsAttribute))
    }

    func testLocalComputerAuthorizationIsResolvedLiveByTheNativeBridge() {
        let authorizations = MacCoreToolPolicy.localHostAuthorizations()

        XCTAssertEqual(authorizations["computer"], "granted")
        XCTAssertEqual(authorizations["clipboard"], "granted")
        XCTAssertEqual(authorizations["files_read"], "granted")
        XCTAssertEqual(authorizations["files_write"], "granted")
        XCTAssertEqual(authorizations["process"], "granted")
    }

    func testPermissionSubjectIdentifiesTheActualRunningExecutable() {
        let subject = MacSystemPermissionProbe.permissionSubject()

        XCTAssertFalse(subject["bundle_id", default: ""].isEmpty)
        XCTAssertFalse(subject["bundle_path", default: ""].isEmpty)
        XCTAssertFalse(subject["executable", default: ""].isEmpty)
        XCTAssertNotNil(subject["is_main_app_process"])
    }

    func testComputerStatusReadsGrantedSystemPermissionsThroughTheNativeBridge() async throws {
        let data = await MacComputerUseService.responseData(
            requestID: "permission-status-e2e",
            toolName: "computer_status",
            arguments: [:]
        )
        let response = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: Any]
        )
        let output = try XCTUnwrap(response["output"] as? [String: Any])
        let subject = try XCTUnwrap(output["permission_subject"] as? [String: String])
        let expectedSubject = MacSystemPermissionProbe.permissionSubject()

        XCTAssertEqual(response["is_error"] as? Bool, false)
        XCTAssertEqual(output["executor"] as? String, "nola_mac_app")
        XCTAssertEqual(output["permission_state_is_live"] as? Bool, true)
        XCTAssertEqual(subject["bundle_id"], expectedSubject["bundle_id"])
        XCTAssertEqual(subject["bundle_path"], expectedSubject["bundle_path"])
        XCTAssertEqual(subject["executable"], expectedSubject["executable"])
        XCTAssertEqual(subject["is_main_app_process"], expectedSubject["is_main_app_process"])
        XCTAssertEqual(
            output["accessibility_permission"] as? Bool,
            MacSystemPermissionProbe.accessibility(),
            "The native bridge must report the live permission of its own running code identity"
        )
        XCTAssertEqual(
            output["accessibility_event_posting_authorized"] as? Bool,
            MacSystemPermissionProbe.accessibilitySnapshot().eventPostingAuthorized,
            "The native bridge must expose the event-posting state allowed by the current runtime policy"
        )
        XCTAssertEqual(
            output["screen_recording_permission"] as? Bool,
            MacSystemPermissionProbe.screenRecordingSnapshot().isAuthorized,
            "The native bridge must report current preflight or observed capture success"
        )
        if ProcessInfo.processInfo.environment["NOLA_REQUIRE_MAC_SYSTEM_PERMISSIONS"] == "1" {
        XCTAssertEqual(output["accessibility_permission"] as? Bool, true)
            XCTAssertEqual(output["accessibility_event_posting_authorized"] as? Bool, true)
            XCTAssertEqual(output["screen_recording_permission"] as? Bool, true)
        }
    }

    func testDefaultXCTestRuntimeSuppressesSystemPermissionPromptsAndUsesEphemeralCredentials() {
        XCTAssertTrue(MacProcessRuntimePolicy.isRunningTests)
        if let explicitRoot = ProcessInfo.processInfo.environment["NOLA_MAC_APP_SUPPORT"]?
            .trimmingCharacters(in: .whitespacesAndNewlines), !explicitRoot.isEmpty {
            XCTAssertEqual(MacAppEnvironment.shared.supportRoot.standardizedFileURL,
                URL(fileURLWithPath: explicitRoot, isDirectory: true).standardizedFileURL)
        } else {
            XCTAssertTrue(MacAppEnvironment.shared.supportRoot.path.contains("Nola-XCTest-"))
        }
        XCTAssertFalse(
            MacAppEnvironment.shared.supportRoot.standardizedFileURL.path
                == FileManager.default.homeDirectoryForCurrentUser
                    .appendingPathComponent("Library/Application Support/Nola")
                    .standardizedFileURL.path
        )
        if ProcessInfo.processInfo.environment["NOLA_REQUIRE_MAC_SYSTEM_PERMISSIONS"] != "1" {
            XCTAssertFalse(MacProcessRuntimePolicy.allowsSystemPermissionPrompts)
        }
        if ProcessInfo.processInfo.environment["NOLA_USE_REAL_KEYCHAIN_IN_TESTS"] != "1" {
            XCTAssertTrue(MacProcessRuntimePolicy.usesEphemeralCredentials)
        }
    }

    func testDecodesSuccessfulRecallContext() {
        let data = Data(#"{"ok":true,"context":"  - relevant memory  "}"#.utf8)

        XCTAssertEqual(
            MacPersonalIntelligenceCore.decodeRecallContext(data),
            "- relevant memory"
        )
    }

    func testRejectsFailedOrMalformedRecallResponse() {
        XCTAssertEqual(
            MacPersonalIntelligenceCore.decodeRecallContext(
                Data(#"{"ok":false,"context":"must not leak"}"#.utf8)
            ),
            ""
        )
        XCTAssertEqual(
            MacPersonalIntelligenceCore.decodeRecallContext(Data("not-json".utf8)),
            ""
        )
    }
}

final class MacAgentHostEventContractTests: XCTestCase {
    func testCoreToolPolicyConsumesSharedGoldenFixture() throws {
        let fixtureURL = try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "tool_policy_v2_golden",
                withExtension: "json"
            )
        )
        let root = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(contentsOf: fixtureURL)) as? [String: Any]
        )
        let input = try XCTUnwrap(root["input"] as? [String: Any])
        let expected = try XCTUnwrap(root["expected"] as? [String: Any])

        let output = try MacCoreToolPolicy.resolveOutputJSON(
            JSONSerialization.data(withJSONObject: input, options: [.sortedKeys])
        )

        XCTAssertEqual(output.schemaVersion, 2)
        XCTAssertEqual(output.enabledTools.map(\.toolId), expected["enabled_tool_ids"] as? [String])
        let expectedDisabled = try XCTUnwrap(expected["disabled_tools"] as? [[String: Any]])
        XCTAssertEqual(output.disabledTools.map(\.toolId), expectedDisabled.compactMap { $0["tool_id"] as? String })
        XCTAssertEqual(output.disabledTools.map(\.reason), expectedDisabled.compactMap { $0["reason"] as? String })
        let receipt = try XCTUnwrap(expected["receipt"] as? [String: Any])
        let digestLength = (receipt["digest_hex_length"] as? NSNumber)?.intValue
        XCTAssertEqual(output.policyDigest.count, digestLength)
        XCTAssertEqual(output.manifestDigest.count, digestLength)
    }

    func testLocalCorePlanUsesFreshIdentityBoundNumericManifest() throws {
        let nowMs: Int64 = 1_000_000
        let plan = try MacCoreToolPolicy.localPlan(
            executableURL: URL(fileURLWithPath: "/bin/echo"),
            projectURL: URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true),
            sessionId: "local-session",
            nowMs: nowMs
        )

        XCTAssertEqual(plan.identity, .init(hostId: "local", clientId: "mac", sessionId: "local-session"))
        XCTAssertTrue(plan.sourceManifestId.hasPrefix("mac-local:"))
        XCTAssertEqual(plan.output.schemaVersion, 2)
        XCTAssertEqual(plan.output.manifestValidUntilMs, nowMs + 300_000)
        XCTAssertEqual(plan.output.policyDigest.count, 64)
        XCTAssertEqual(plan.output.manifestDigest.count, 64)
        XCTAssertFalse(plan.output.enabledTools.isEmpty)
        for toolId in ["terminal_input", "language_service_list", "language_service_query",
                       "append_file", "git_commit", "environment_probe", "skill_update", "skill_skip",
                       "spawn_agent", "list_tasks", "read_task", "wait_task", "stop_task", "send_agent_message"] {
            XCTAssertTrue(plan.output.enabledTools.contains { $0.toolId == toolId },
                          "Native tool plan must retain the shared Host capability: \(toolId)")
        }
        XCTAssertNotNil(plan.resolvedPermissionTool(
            toolId: "workspace_access", hostId: "local", clientId: "mac",
            sessionId: "local-session",
            policyDigest: plan.output.policyDigest,
            manifestDigest: plan.output.manifestDigest, nowMs: nowMs
        ), "External workspace requests must authenticate against the native Core tool plan.")
        XCTAssertEqual(
            plan.output.enabledTools.first { $0.toolId == "read_file" }?.authorization,
            "granted"
        )
        XCTAssertEqual(
            plan.output.enabledTools.first { $0.toolId == "write_file" }?.authorization,
            "granted"
        )
        XCTAssertEqual(
            plan.output.enabledTools.first { $0.toolId == "skill" }?.authorization,
            "granted"
        )
        XCTAssertNil(plan.output.enabledTools.first { $0.toolId == "skill_context" })
        XCTAssertNil(
            plan.resolvedPermissionTool(
                toolId: "write_file",
                hostId: "local",
                clientId: "mac",
                sessionId: "local-session",
                nowMs: nowMs + 600_000
            )
        )
        XCTAssertTrue(
            plan.matchesPermissionChallenge(
                toolId: "write_file",
                hostId: "local",
                clientId: "mac",
                sessionId: "local-session",
                policyDigest: plan.output.policyDigest,
                manifestDigest: plan.output.manifestDigest
            ),
            "An expired capability snapshot must still authenticate its original pending challenge before a fresh snapshot revalidates the tool."
        )
        XCTAssertFalse(
            plan.matchesPermissionChallenge(
                toolId: "write_file",
                hostId: "local",
                clientId: "mac",
                sessionId: "local-session",
                policyDigest: nil,
                manifestDigest: nil
            ),
            "Permission challenges without current policy and manifest digests must be rejected."
        )
        XCTAssertFalse(
            plan.matchesPermissionChallenge(
                toolId: "write_file",
                hostId: "local",
                clientId: "mac",
                sessionId: "local-session",
                policyDigest: "mismatched",
                manifestDigest: plan.output.manifestDigest
            )
        )
        XCTAssertNil(
            plan.resolvedPermissionTool(
                toolId: plan.output.enabledTools[0].toolId,
                hostId: "local",
                clientId: "mac",
                sessionId: "wrong-session",
                nowMs: nowMs
            )
        )
    }

    func testResourceScopedRecallChallengeDoesNotRequireStaticPermissionDomain() throws {
        let now: Int64 = 1_000_000
        let plan = try MacCoreToolPolicy.localPlan(executableURL: URL(fileURLWithPath: "/bin/echo"),
            projectURL: URL(fileURLWithPath: NSTemporaryDirectory()), sessionId: "recall-owner", nowMs: now)
        XCTAssertEqual(plan.output.enabledTools.first { $0.toolId == "conversation_recall" }?.permission, "none")
        XCTAssertTrue(plan.matchesPermissionChallenge(toolId: "conversation_recall", hostId: "local",
            clientId: "mac", sessionId: "recall-owner", policyDigest: plan.output.policyDigest,
            manifestDigest: plan.output.manifestDigest))
        XCTAssertNotNil(plan.resolvedPermissionTool(toolId: "conversation_recall", hostId: "local",
            clientId: "mac", sessionId: "recall-owner", policyDigest: plan.output.policyDigest,
            manifestDigest: plan.output.manifestDigest, nowMs: now))
        for toolID in ["conversation_recall", "unknown_tool"] {
            XCTAssertFalse(plan.matchesPermissionChallenge(toolId: toolID, hostId: "local",
                clientId: "mac", sessionId: "other-owner", policyDigest: plan.output.policyDigest,
                manifestDigest: plan.output.manifestDigest))
        }
        XCTAssertFalse(plan.matchesPermissionChallenge(toolId: "unknown_tool", hostId: "local",
            clientId: "mac", sessionId: "recall-owner", policyDigest: plan.output.policyDigest,
            manifestDigest: plan.output.manifestDigest))
        XCTAssertFalse(plan.matchesPermissionChallenge(toolId: "conversation_recall", hostId: "local",
            clientId: "mac", sessionId: "recall-owner", policyDigest: "tampered",
            manifestDigest: plan.output.manifestDigest))
    }

    func testRemoteToolPlanUsesActualHostSchemasAndIdentity() throws {
        let snapshot: [String: Any] = [
            "schema_version": 1,
            "reference": ["manifest_id": "host-issued", "host_id": "linux-host", "client_id": "mac-client",
                          "session_id": "session", "catalog_revision": String(repeating: "a", count: 64)],
            "issued_at_ms": 1_000, "expires_at_ms": 3_000,
            "tools": [["tool_id": "custom_mcp_query", "route": "agent_host",
                       "schema": ["type": "function", "function": ["name": "custom_mcp_query",
                           "description": "Host-provided full schema", "parameters": ["type": "object", "required": ["query"]]]]]]
        ]
        let plan = try JSONDecoder().decode(MacRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: snapshot))
        try plan.validate(hostID: "linux-host", clientID: "mac-client", sessionID: "session")
        XCTAssertEqual(plan.tools.map(\.tool_id), ["custom_mcp_query"])
        XCTAssertEqual(plan.referenceObject["host_id"] as? String, "linux-host")
        XCTAssertThrowsError(try plan.validate(hostID: "mac-host", clientID: "mac-client", sessionID: "session"))
        XCTAssertThrowsError(try plan.validate(hostID: "linux-host", clientID: "other", sessionID: "session"))
        XCTAssertThrowsError(try plan.validate(hostID: "linux-host", clientID: "mac-client", sessionID: "other"))
        // Recovering the original authenticated Run does not mint a new manifest.
        try plan.validate(hostID: "linux-host", clientID: "mac-client", sessionID: "session")
        for (key, value) in [("legacy", true as Any), ("schema_version", "1" as Any), ("schema_version", true as Any),
                             ("issued_at_ms", "1000" as Any), ("expires_at_ms", "3000" as Any),
                             ("issued_at_ms", 0 as Any), ("issued_at_ms", 3000 as Any),
                             ("expires_at_ms", 301001 as Any), ("expires_at_ms", 9_007_199_254_740_992 as Any)] {
            var changed = snapshot; changed[key] = value
            XCTAssertThrowsError(try JSONDecoder().decode(MacRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: changed))
                .validate(hostID: "linux-host", clientID: "mac-client", sessionID: "session"), key)
        }
        // Both long-past and far-future Host timestamps are structurally valid;
        // freshness is checked only by the execution Host, not this device clock.
        for issued: Int64 in [1_000, 8_000_000_000_000_000] {
            var timed = snapshot; timed["issued_at_ms"] = issued; timed["expires_at_ms"] = issued + 300_000
            _ = try JSONDecoder().decode(MacRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: timed))
                .validate(hostID: "linux-host", clientID: "mac-client", sessionID: "session")
        }
        let originalTools = try XCTUnwrap(snapshot["tools"] as? [[String: Any]])
        var duplicate = snapshot; duplicate["tools"] = originalTools + originalTools
        var foreign = snapshot
        var originalReference = try XCTUnwrap(snapshot["reference"] as? [String: Any])
        originalReference["legacy"] = true; foreign["reference"] = originalReference
        var malformedTool = originalTools[0]; malformedTool["legacy"] = true
        var unknownTool = snapshot; unknownTool["tools"] = [malformedTool]
        for changed in [duplicate, foreign, unknownTool] {
            XCTAssertThrowsError(try JSONDecoder().decode(MacRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: changed))
                .validate(hostID: "linux-host", clientID: "mac-client", sessionID: "session"))
        }
        for invalid in [" \n", "id\u{0000}suffix", "id\u{0085}suffix", String(repeating: "x", count: 513)] {
            var changed = snapshot
            var reference = try XCTUnwrap(snapshot["reference"] as? [String: Any]); reference["manifest_id"] = invalid
            changed["reference"] = reference
            XCTAssertThrowsError(try JSONDecoder().decode(MacRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: changed))
                .validate(hostID: "linux-host", clientID: "mac-client", sessionID: "session"))
        }
    }

    func testRemotePermissionAcceptsCurrentCoreDescriptorWithoutInitialCatalogMembership() throws {
        let evidence: [String: Any] = ["catalog_revision": 7,
            "tool": ["tool_id": "mcp_newly_connected", "route": "agent_host",
                     "schema": ["type": "function", "function": ["name": "mcp_newly_connected", "parameters": ["type": "object"]]]]]
        let parsed = try XCTUnwrap(MacRemotePermissionToolEvidence.decode(evidence, toolName: "mcp_newly_connected"))
        XCTAssertEqual(parsed.catalogRevision, 7)
        XCTAssertNil(MacRemotePermissionToolEvidence.decode(evidence, toolName: "another_tool"))
        XCTAssertNil(MacRemotePermissionToolEvidence.decode(nil, toolName: "mcp_newly_connected"))
        for invalid: Any in ["7", true, -1] {
            var changed = evidence; changed["catalog_revision"] = invalid
            XCTAssertNil(MacRemotePermissionToolEvidence.decode(changed, toolName: "mcp_newly_connected"))
        }
        var unknown = evidence; unknown["legacy"] = true
        XCTAssertNil(MacRemotePermissionToolEvidence.decode(unknown, toolName: "mcp_newly_connected"))
    }

    func testDecodesProductionCLIToolLifecycleWithStableIdentity() throws {
        let started = try XCTUnwrap(
            MacAgentHostRuntime.decode(
                line: traceLine(
                    kind: "tool.started",
                    eventID: "event-start-42",
                    callID: "call-42",
                    toolName: "read_file",
                    status: "running",
                    arguments: ["path": "README.md", "line_start": 1]
                )
            )
        )
        XCTAssertEqual(started.kind, .toolStarted)
        XCTAssertEqual(started.eventID, "event-start-42")
        XCTAssertEqual(started.toolCallID, "call-42")
        XCTAssertEqual(started.title, "read_file")
        let detailData = try XCTUnwrap(started.detail?.data(using: .utf8))
        let detail = try XCTUnwrap(
            JSONSerialization.jsonObject(with: detailData) as? [String: AnyHashable]
        )
        XCTAssertEqual(detail, ["line_start": 1, "path": "README.md"])
        XCTAssertFalse(started.isError)

        let finished = try XCTUnwrap(
            MacAgentHostRuntime.decode(
                line: traceLine(
                    kind: "tool.completed",
                    eventID: "event-finish-42",
                    callID: "call-42",
                    toolName: "read_file",
                    status: "failed",
                    summary: "Read README.md"
                )
            )
        )
        XCTAssertEqual(finished.kind, .toolFinished)
        XCTAssertEqual(finished.eventID, "event-finish-42")
        XCTAssertEqual(finished.toolCallID, started.toolCallID)
        XCTAssertEqual(finished.title, "read_file")
        XCTAssertEqual(finished.text, "Read README.md")
        XCTAssertEqual(finished.detail, "Read README.md")
        XCTAssertTrue(finished.isError)
    }

    func testDecodesToolLifecycleWithoutLosingIdentity() throws {
        let started = try XCTUnwrap(
            MacAgentHostRuntime.decode(
                line: traceLine(
                    kind: "tool.started",
                    eventID: "event-browser-start",
                    callID: "call-browser",
                    toolName: "browser.open",
                    status: "running",
                    summary: "Opening page"
                )
            )
        )
        XCTAssertEqual(started.kind, .toolStarted)
        XCTAssertEqual(started.eventID, "event-browser-start")
        XCTAssertEqual(started.title, "browser.open")
        XCTAssertEqual(started.detail, "Opening page")
        XCTAssertFalse(started.isError)

        let finished = try XCTUnwrap(
            MacAgentHostRuntime.decode(
                line: traceLine(
                    kind: "tool.completed",
                    eventID: "event-browser-finish",
                    callID: "call-browser",
                    toolName: "browser.open",
                    status: "failed",
                    summary: "failed"
                )
            )
        )
        XCTAssertEqual(finished.kind, .toolFinished)
        XCTAssertFalse(finished.eventID.isEmpty)
        XCTAssertEqual(finished.title, "browser.open")
        XCTAssertTrue(finished.isError)
    }

    func testPreservesExplicitEventAndToolCallIdentifiers() throws {
        let event = try XCTUnwrap(
            MacAgentHostRuntime.decode(
                line: traceLine(
                    kind: "tool.started",
                    eventID: "event-7",
                    callID: "call-7",
                    toolName: "search",
                    status: "running"
                )
            )
        )

        XCTAssertEqual(event.eventID, "event-7")
        XCTAssertEqual(event.toolCallID, "call-7")
    }

    func testPlainStatusLineRemainsVisible() throws {
        let status = try XCTUnwrap(MacAgentHostRuntime.decode(line: "Preparing local host"))
        XCTAssertEqual(status.kind, .status)
        XCTAssertEqual(status.text, "Preparing local host")
    }

    private func traceLine(
        kind: String,
        eventID: String,
        callID: String,
        toolName: String,
        status: String,
        summary: String? = nil,
        arguments: [String: Any]? = nil
    ) -> String {
        var payload: [String: Any] = [
            "tool_name": toolName,
            "call_id": callID,
        ]
        if let summary {
            payload["summary"] = summary
        }
        if let arguments {
            payload["arguments"] = arguments
        }
        let object: [String: Any] = [
            "trace": [
                "schema_version": 1,
                "trace_id": "trace-contract",
                "run_id": "run-contract",
                "conversation_id": "local",
                "turn_id": "turn-contract",
                "event_id": eventID,
                "sequence": 1,
                "emitted_at": "2026-07-19T00:00:00Z",
                "kind": kind,
                "span_id": callID,
                "status": status,
                "visibility": "summary",
                "payload": payload,
            ],
        ]
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }
}
