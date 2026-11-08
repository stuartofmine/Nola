// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements WebAgentProviderTests logic in the project module.
// - Interfaces:
//   - WebAgentProviderTests
// - APIs:
//   - none
//
import XCTest
@testable import Nola

@MainActor
final class WebAgentProviderTests: XCTestCase {
    func testSharedSSEByteFramerPreservesEmptyFramesAndSplitUnicode() {
        let input = Array("data: 第一🙂\r\n\r\ndata: 第二\n\ndata: tail".utf8)
        for chunkSize in [1, 2, 3, 7, input.count] {
            var framer = WebAgentSSELineFramer()
            var lines: [String] = []
            for start in stride(from: 0, to: input.count, by: chunkSize) {
                for byte in input[start..<min(start + chunkSize, input.count)] {
                    if let line = framer.append(byte) { lines.append(line) }
                }
            }
            if let line = framer.finish() { lines.append(line) }
            XCTAssertEqual(lines, ["data: 第一🙂\r", "\r", "data: 第二", "", "data: tail"])
            XCTAssertNil(framer.finish())
        }
    }

    func testDelegatedPermissionTransportDoesNotRequireOrInventParentTrace() {
        var envelope: [String: Any] = [
            "type": "permission_request", "task_id": "child-task",
            "protocol_version": 4, "allow_conversation": false,
            "id": "child-request", "tool_name": "bash",
            "nonce": "challenge", "host_id": "host", "client_id": "client",
            "session_id": "parent", "canonical_run_id": "parent-run",
            "expires_at": 4_000_000_000, "permission_instructions": [],
            "safe_review_text": "Run the delegated command",
        ]
        guard case .delegatedPermissionRequest(let request)? = WebAgentProvider.parseSSEObject(envelope) else {
            return XCTFail("A real delegated Host challenge has no parent Core trace")
        }
        XCTAssertEqual(request.canonicalRunID, "parent-run")
        XCTAssertEqual(request.id, "child-request")
        XCTAssertFalse(request.allowConversation)
        envelope["protocol_version"] = 3
        guard case .error? = WebAgentProvider.parseSSEObject(envelope) else { return XCTFail("Reject wrong protocol") }
        envelope["protocol_version"] = 4
        envelope.removeValue(forKey: "allow_conversation")
        guard case .error? = WebAgentProvider.parseSSEObject(envelope) else { return XCTFail("Do not infer eligibility") }
    }

    func testUISourceRequiresBoundEndpointPinAndPairedIdentity() throws {
        let url = try XCTUnwrap(URL(string: "https://host.example"))
        let pin = String(repeating: "ab", count: 32)
        let provider = WebAgentProvider(config: .init(baseURL: url, token: "fixture",
            pinnedCertificateSHA256: pin, expectedHostId: "host-a", clientId: "fixture-client"))
        XCTAssertTrue(provider.matchesUIHost(baseURL: url.absoluteString, certificateSHA256: pin))
        XCTAssertFalse(provider.matchesUIHost(baseURL: "https://other.example", certificateSHA256: pin))
        XCTAssertFalse(provider.matchesUIHost(baseURL: url.absoluteString, certificateSHA256: nil))
        let unpaired = WebAgentProvider(config: .init(baseURL: url, token: "fixture", clientId: "fixture-client"))
        XCTAssertFalse(unpaired.matchesUIHost(baseURL: url.absoluteString, certificateSHA256: nil))
    }

    func testExecutionBackendAvailabilityUsesCoreWireContract() throws {
        func decode(_ availability: String) throws -> WebAgentProvider.HostAgentSettings.ExecutionBackend {
            let data = try JSONSerialization.data(withJSONObject: [
                "id": "external",
                "display_name": "External",
                "availability": availability,
                "capabilities": [
                    "streams_text": true,
                    "streams_reasoning": true,
                    "streams_tools": true,
                    "supports_resume": true,
                    "supports_cancel": true,
                    "supports_permission_bridge": false,
                ],
            ])
            return try JSONDecoder().decode(
                WebAgentProvider.HostAgentSettings.ExecutionBackend.self,
                from: data
            )
        }

        XCTAssertEqual(try decode("available").availabilityState, .available)
        XCTAssertEqual(try decode("not_installed").availabilityState, .notInstalled)
        XCTAssertEqual(try decode("unauthenticated").availabilityState, .signInRequired)
        XCTAssertEqual(try decode("incompatible").availabilityState, .incompatible)
        XCTAssertEqual(try decode("unreachable").availabilityState, .unreachable)
        XCTAssertEqual(try decode("sign_in_required").availabilityState, .unavailable)
        XCTAssertEqual(try decode("incompatible_version").availabilityState, .unavailable)
    }

    func testParseCoreContextUsageEvent() throws {
        let event = WebAgentProvider.parseSSEObject(traced(
            kind: "usage.snapshot",
            payload: [
                "usage": [
                    "used_tokens": 9_000,
                    "context_window_tokens": 12_000,
                    "prompt_token_limit": 12_000,
                ],
            ]
        ))

        guard case .trace(let trace)? = event else {
            return XCTFail("Expected usage trace")
        }
        XCTAssertEqual(trace.kind, .usageSnapshot)
        XCTAssertEqual(trace.payload.usage?.usedTokens, 9_000)
        XCTAssertEqual(trace.payload.usage?.contextWindowTokens, 12_000)
    }

    func testMobileAuthHeadersReplaceBearerToken() throws {
        let url = try XCTUnwrap(URL(string: "https://host.example/api/mobile/agent/settings"))
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.setValue("Bearer exposed-token", forHTTPHeaderField: "Authorization")

        WebAgentProvider.applyMobileAuth(to: &request, token: "shared-token")

        XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
        XCTAssertNotNil(request.value(forHTTPHeaderField: "X-Nola-Mobile-Timestamp"))
        XCTAssertNotNil(request.value(forHTTPHeaderField: "X-Nola-Mobile-Nonce"))
        XCTAssertEqual(request.value(forHTTPHeaderField: "X-Nola-Mobile-Body-SHA256")?.count, 64)
        XCTAssertNotNil(request.value(forHTTPHeaderField: "X-Nola-Mobile-Auth"))
    }

    func testBrowserHandoffPathUsesHostOriginWithoutLeakingMobileAPIPath() throws {
        let baseURL = try XCTUnwrap(URL(string: "https://host.example:7443/api/mobile/agent/settings"))
        let result = try XCTUnwrap(WebAgentProvider.controlPlaneURL(
            baseURL: baseURL,
            path: "/auth/mobile-handoff/one-time-code"
        ))
        XCTAssertEqual(result.absoluteString, "https://host.example:7443/auth/mobile-handoff/one-time-code")
    }

    func testParseToolUseEvent() throws {
        let event = WebAgentProvider.parseSSEObject(traced(
            kind: "tool.started",
            spanID: "call_1",
            payload: [
                "tool_name": "chrome_native_open_page",
                "call_id": "call_1",
                "arguments": ["url": "https://www.baidu.com/s?wd=opencode"],
            ]
        ))

        guard case .trace(let trace)? = event else {
            return XCTFail("Expected tool trace")
        }
        XCTAssertEqual(trace.kind, .toolStarted)
        XCTAssertEqual(trace.spanId, "call_1")
        XCTAssertEqual(trace.payload.toolName, "chrome_native_open_page")
        XCTAssertEqual(trace.payload.callId, "call_1")
    }

    func testParseBrowserToolCompletionTrace() throws {
        let event = WebAgentProvider.parseSSEObject(traced(
            kind: "tool.completed",
            status: "succeeded",
            spanID: "call_1",
            payload: [
                "summary": "百度搜索：opencode",
                "tool_name": "chrome_native_open_page",
                "call_id": "call_1",
                "result": ["text": "opened"],
                "resources": [
                    [
                        "uri": "https://www.baidu.com/s?wd=opencode",
                        "operation": "open",
                        "display_name": "百度一下",
                        "mime_type": "text/html",
                    ],
                    [
                        "uri": "file:///tmp/export.csv",
                        "operation": "create",
                        "display_name": "export.csv",
                        "mime_type": "text/csv",
                    ],
                ],
            ]
        ))

        guard case .trace(let trace)? = event else {
            return XCTFail("Expected tool completion trace")
        }
        XCTAssertEqual(trace.kind, .toolCompleted)
        XCTAssertEqual(trace.status, .succeeded)
        XCTAssertEqual(trace.payload.summary, "百度搜索：opencode")
        XCTAssertEqual(trace.payload.resources.count, 2)
        XCTAssertEqual(trace.payload.resources.first?.operation, .open)
        XCTAssertEqual(trace.payload.resources.last?.operation, .create)
    }

    func testParseComputerToolCompletionTrace() throws {
        let event = WebAgentProvider.parseSSEObject(traced(
            kind: "tool.completed",
            status: "succeeded",
            spanID: "call_screen",
            payload: [
                "summary": "电脑操作完成：screenshot",
                "tool_name": "computer_screenshot",
                "call_id": "call_screen",
                "result": ["text": "Screenshot saved"],
                "resources": [
                    [
                        "uri": "file:///tmp/call_screen.png",
                        "operation": "create",
                        "display_name": "call_screen.png",
                        "mime_type": "image/png",
                    ],
                ],
            ]
        ))

        guard case .trace(let trace)? = event else {
            return XCTFail("Expected computer tool completion trace")
        }
        XCTAssertEqual(trace.payload.toolName, "computer_screenshot")
        XCTAssertEqual(trace.payload.resources.first?.mimeType, "image/png")
    }

    func testParsePermissionRequiredToolResultAndRequest() throws {
        let requestEvent = WebAgentProvider.parseSSEObject(traced([
            "type": "permission_request", "allow_conversation": true,
            "protocol_version": 4,
            "id": "call_disk",
            "name": "disk_usage",
            "match": "broad:/",
            "reason": "Scanning / may be slow",
            "args": ["path": "/"]
        ], kind: "tool.permission.requested", status: "waiting", spanID: "call_disk", payload: [
            "tool_name": "disk_usage",
            "call_id": "call_disk",
            "permission_request_id": "call_disk",
            "permission_scope": "broad:/",
            "reason": "Scanning / may be slow",
        ]))
        guard case .permissionRequest(let request, let trace)? = requestEvent else {
            return XCTFail("Expected permission request")
        }
        XCTAssertEqual(request.id, "call_disk")
        XCTAssertEqual(request.toolName, "disk_usage")
        XCTAssertEqual(request.match, "broad:/")
        XCTAssertTrue(request.argsPreview.contains("\"path\""))
        XCTAssertEqual(trace.kind, .toolPermissionRequested)
        XCTAssertEqual(trace.status, .waiting)
    }

    func testPermissionInstructionEnvelopeKeepsOriginalArgumentsAndRejectsMalformedRecords() throws {
        var object: [String: Any] = ["type": "permission_request", "allow_conversation": true, "protocol_version":  4, "id": "new-request", "name": "write_file",
            "args": ["path": "/original"],
            "permission_instructions": [["permission_request_id": "old-request", "instruction": "Preview first."]]]
        func parse(_ value: [String: Any]) -> WebAgentStreamEvent? {
            WebAgentProvider.parseSSEObject(traced(value, kind: "tool.permission.requested", status: "waiting", spanID: "same-call", payload: ["tool_name": "write_file", "call_id": "same-call", "permission_request_id": "new-request"]))
        }
        guard case .permissionRequest(let request, _)? = parse(object) else { return XCTFail("Expected permission request") }
        XCTAssertEqual(request.permissionInstructions?.first?.permissionRequestId, "old-request")
        XCTAssertEqual(request.permissionInstructions?.first?.instruction, "Preview first.")
        XCTAssertTrue(request.allowConversation)
        var once = object; once["allow_conversation"] = false
        guard case .permissionRequest(let onceRequest, _)? = parse(once) else { return XCTFail("Expected once-only request") }
        XCTAssertFalse(onceRequest.allowConversation)
        once.removeValue(forKey: "allow_conversation")
        guard case .error? = parse(once) else { return XCTFail("Missing eligibility must fail closed") }
        XCTAssertTrue(request.argsPreview.contains("/original"))
        XCTAssertFalse(request.argsPreview.contains("Preview"))
        object["permission_instructions"] = [["permission_request_id": "old-request", "instruction": 42]]
        guard case .error? = parse(object) else { return XCTFail("Malformed guidance must not disappear") }
    }

    func testHostToolEventsConversationMetaRoundtrip() throws {
        let events = [
            HostAgentToolEvent(
                id: "call_1",
                name: "chrome_native_open_page",
                argsPreview: "{\"url\":\"https://example.com\"}",
                status: .succeeded,
                display: "Opened page, screenshot saved",
                text: "Visible text",
                screenshotURL: URL(string: "/static/agent_artifacts/browser/user/call_1.png"),
                finalURL: URL(string: "https://example.com"),
                pageTitle: "Example",
                downloadURL: URL(string: "/static/agent_artifacts/browser/user/downloads/export.csv"),
                downloadFilename: "export.csv",
                screenshotWarning: "screenshot too large"
            )
        ]

        let encoded = try XCTUnwrap(HostAgentToolEvent.encodeForConversationMeta(events))
        let decoded = HostAgentToolEvent.decodeFromConversationMeta([
            HostAgentToolEvent.conversationMetaKey: encoded
        ])

        XCTAssertEqual(decoded, events)
    }

    func testHostToolEventsConversationMetaRedactsSensitiveArtifacts() throws {
        let events = [
            HostAgentToolEvent(
                id: "call_secret",
                name: "chrome_native_open_page",
                status: .succeeded,
                display: "Opened callback access_token=secret-value",
                text: "Saved access_token=secret-value",
                screenshotURL: URL(string: "/static/agent_artifacts/browser/user/call.png"),
                finalURL: URL(string: "https://example.com/callback?access_token=secret-value"),
                pageTitle: "Callback access_token=secret-value",
                downloadURL: URL(string: "/static/agent_artifacts/browser/user/downloads/export.csv?access_token=secret-value"),
                downloadFilename: "export-access_token=secret-value.csv",
                screenshotWarning: "Screenshot failed token=secret-value"
            )
        ]

        let encoded = try XCTUnwrap(HostAgentToolEvent.encodeForConversationMeta(events))
        let decoded = HostAgentToolEvent.decodeFromConversationMeta([
            HostAgentToolEvent.conversationMetaKey: encoded
        ])

        XCTAssertEqual(decoded.count, 1)
        XCTAssertEqual(decoded[0].display, "Opened callback access_token=[已隐藏]")
        XCTAssertEqual(decoded[0].text, "")
        XCTAssertNil(decoded[0].screenshotURL)
        XCTAssertNil(decoded[0].finalURL)
        XCTAssertNil(decoded[0].pageTitle)
        XCTAssertNil(decoded[0].downloadURL)
        XCTAssertNil(decoded[0].downloadFilename)
        XCTAssertEqual(decoded[0].screenshotWarning, "Screenshot failed token=[已隐藏]")
        XCTAssertTrue(decoded[0].sensitive)
    }

    func testNativeChromeToolsAreBrowserToolsAndSensitiveTextIsRedacted() throws {
        let event = HostAgentToolEvent(
            id: "call_secret",
            name: "chrome_native_get_page_state",
            status: .succeeded,
            display: "OAuth client secret: GOCSPX-secret-value",
            text: #"{"client_secret":"sk-test_abcdefghijklmnopqrstuvwxyz","token":"secret-value","authorization":"Bearer abcdefghijk12345","session":"session-secret-value"}"#,
            screenshotWarning: "Screenshot failed token=secret-value"
        )

        XCTAssertTrue(event.isBrowserTool)
        XCTAssertTrue(event.containsSensitiveData)
        XCTAssertFalse(event.redactedDisplay.contains("secret-value"))
        XCTAssertFalse(event.redactedText.contains("abcdefghijklmnopqrstuvwxyz"))
        XCTAssertFalse(event.redactedText.contains("abcdefghijk12345"))
        XCTAssertFalse(event.redactedText.contains("session-secret-value"))
        XCTAssertFalse(event.redactedScreenshotWarning.contains("secret-value"))
        XCTAssertTrue(event.redactedText.contains(#""authorization":"[已隐藏]""#))
        XCTAssertTrue(HostAgentToolEvent.containsSensitiveData(in: #"{"token":"json-secret-value","secret":"json-second-secret","session":"json-session-secret"}"#))
        let redactedJSON = HostAgentToolEvent.redactSensitiveContent(#"{"token":"json-secret-value","secret":"json-second-secret","session":"json-session-secret"}"#)
        XCTAssertFalse(redactedJSON.contains("json-secret-value"))
        XCTAssertFalse(redactedJSON.contains("json-second-secret"))
        XCTAssertFalse(redactedJSON.contains("json-session-secret"))
    }

    func testHostToolSensitiveDetectionIncludesArtifactURLs() throws {
        let event = HostAgentToolEvent(
            id: "call_token_url",
            name: "chrome_native_open_page",
            status: .succeeded,
            finalURL: URL(string: "https://example.com/callback?access_token=secret-token")
        )

        XCTAssertTrue(event.containsSensitiveData)
    }

    func testSummaryStreamToolResultLineHidesSensitiveResults() throws {
        let event = HostAgentToolEvent(
            id: "call_clipboard",
            name: "computer_get_clipboard",
            status: .succeeded,
            display: "Clipboard: plain clipboard value",
            text: "plain clipboard value",
            sensitive: true
        )

        let line = WebAgentProvider.summaryStreamToolResultLine(for: event)
        let hiddenText = NSLocalizedString(
            "chat.web_agent.summary.sensitive_hidden",
            value: "Sensitive tool result hidden",
            comment: ""
        )

        XCTAssertTrue(line.contains("computer_get_clipboard"))
        XCTAssertTrue(line.contains(hiddenText))
        XCTAssertFalse(line.contains("plain clipboard value"))
        XCTAssertFalse(line.contains("Clipboard"))
    }

    func testSummaryStreamToolResultLineKeepsNonSensitiveSummary() throws {
        let event = HostAgentToolEvent(
            id: "call_page",
            name: "chrome_native_open_page",
            status: .succeeded,
            display: "Opened Example page",
            text: "Visible text"
        )

        let line = WebAgentProvider.summaryStreamToolResultLine(for: event)
        let hiddenText = NSLocalizedString(
            "chat.web_agent.summary.sensitive_hidden",
            value: "Sensitive tool result hidden",
            comment: ""
        )

        XCTAssertTrue(line.contains("chrome_native_open_page"))
        XCTAssertTrue(line.contains("Opened Example page"))
        XCTAssertFalse(line.contains(hiddenText))
    }

    func testSummaryStreamFallbackLinesAreLocalized() throws {
        let runningLine = WebAgentProvider.summaryStreamToolUseLine(for: "chrome_native_open_page")
        let permissionLine = WebAgentProvider.summaryStreamPermissionLine(
            toolName: "disk_usage",
            summary: "Scanning / may be slow"
        )
        let errorLine = WebAgentProvider.summaryStreamErrorLine(for: "authorization: Bearer abcdefghijk12345")
        let runningFormat = NSLocalizedString(
            "chat.web_agent.summary.tool_running_format",
            value: "\n\n🔧 %@ is running…\n",
            comment: ""
        )
        let permissionFormat = NSLocalizedString(
            "chat.web_agent.summary.permission_required_format",
            value: "\n⚠️ %@ requires permission: %@\n",
            comment: ""
        )
        let errorFormat = NSLocalizedString(
            "chat.web_agent.summary.error_format",
            value: "\nError: %@\n",
            comment: ""
        )

        XCTAssertEqual(
            runningLine,
            String(format: runningFormat, locale: Locale.current, "chrome_native_open_page")
        )
        XCTAssertEqual(
            permissionLine,
            String(format: permissionFormat, locale: Locale.current, "disk_usage", "Scanning / may be slow")
        )
        XCTAssertTrue(runningLine.contains("chrome_native_open_page"))
        XCTAssertTrue(permissionLine.contains("disk_usage"))
        XCTAssertTrue(permissionLine.contains("Scanning / may be slow"))
        XCTAssertEqual(
            errorLine,
            String(format: errorFormat, locale: Locale.current, "authorization: [已隐藏]")
        )
        XCTAssertFalse(errorLine.contains("abcdefghijk12345"))
    }

    func testHostAgentSettingsDecodeComputerUseStatus() throws {
        let data = #"""
        {
          "enable_local_dev_tools": "false",
          "uploaded_skills": [
            {
              "name": "Browser Ops",
              "content": "",
              "text": "Use browser tools.",
              "enabled": "false"
            },
            "bad skill entry"
          ],
          "uploaded_documents": [
            {
              "name": "Project Brief",
              "content": "",
              "text": "Nola is local-first.",
              "enabled": "0"
            }
          ],
          "mcp_servers": [
            {
              "name": "Docs MCP",
              "endpoint": "",
              "url": "https://mcp.example.com/docs",
              "enabled": "yes"
            }
          ],
          "native_browser": {
            "token_configured": "true",
            "connected": 1,
            "hosts": [
              {
                "host_id": "chrome-main",
                "capabilities": ["status", "inspect"],
                "last_seen_age_sec": 0.1
              }
            ],
            "tools": ["chrome_native_status", "chrome_native_inspect"],
            "pending_commands": 2,
            "in_flight_commands": 1
          },
          "computer_use": {
            "feature_enabled": "true",
            "token_configured": 1,
            "connected": "on",
            "hosts": [
              {
                "host_id": "macbook-main",
                "capabilities": ["driver:computer_use"],
                "last_seen_age_sec": 0.2
              }
            ],
            "tools": ["computer_status", "computer_screenshot"]
          },
          "messaging": {
            "worker": { "enabled": "true" },
            "management_path": "/settings#channels",
            "channels": [
              {
                "connection_id": "telegram-primary",
                "kind": "telegram",
                "display_name": "Personal bot",
                "enabled": 1,
                "state": "connected",
                "last_received_at": "2026-08-13T00:00:00Z"
              },
              "bad channel"
            ]
          }
        }
        """#.data(using: .utf8)!

        let settings = try JSONDecoder().decode(WebAgentProvider.HostAgentSettings.self, from: data)

        XCTAssertEqual(settings.enable_local_dev_tools, false)
        XCTAssertEqual(settings.uploaded_skills.count, 1)
        XCTAssertEqual(settings.uploaded_skills[0].name, "Browser Ops")
        XCTAssertEqual(settings.uploaded_skills[0].content, "Use browser tools.")
        XCTAssertEqual(settings.uploaded_skills[0].enabled, false)
        XCTAssertEqual(settings.uploaded_documents.first?.content, "Nola is local-first.")
        XCTAssertEqual(settings.uploaded_documents.first?.enabled, false)
        XCTAssertEqual(settings.mcp_servers.first?.endpoint, "https://mcp.example.com/docs")
        XCTAssertEqual(settings.mcp_servers.first?.enabled, true)
        XCTAssertEqual(settings.computer_use?.feature_enabled, true)
        XCTAssertEqual(settings.computer_use?.token_configured, true)
        XCTAssertEqual(settings.computer_use?.connected, true)
        XCTAssertEqual(settings.computer_use?.hosts?.first?.host_id, "macbook-main")
        XCTAssertEqual(settings.computer_use?.hosts?.first?.capabilities?.contains("driver:computer_use"), true)
        XCTAssertEqual(settings.computer_use?.tools?.contains("computer_screenshot"), true)
        XCTAssertEqual(settings.native_browser?.token_configured, true)
        XCTAssertEqual(settings.native_browser?.connected, true)
        XCTAssertEqual(settings.native_browser?.hosts?.first?.host_id, "chrome-main")
        XCTAssertEqual(settings.native_browser?.hosts?.first?.capabilities?.contains("inspect"), true)
        XCTAssertEqual(settings.native_browser?.tools, ["chrome_native_status", "chrome_native_inspect"])
        XCTAssertEqual(settings.native_browser?.pending_commands, 2)
        XCTAssertEqual(settings.native_browser?.in_flight_commands, 1)
        XCTAssertEqual(settings.messaging?.worker.enabled, true)
        XCTAssertEqual(settings.messaging?.channels.count, 1)
        XCTAssertEqual(settings.messaging?.channels.first?.id, "telegram-primary")
        XCTAssertEqual(settings.messaging?.channels.first?.state, "connected")
        XCTAssertEqual(settings.messaging?.management_path, "/settings#channels")
    }

    func testRetiredBodyTracesCannotEnterControlStreamAndTerminalEventsRemain() throws {
        for kind in ["assistant.delta", "assistant.reset", "thinking.delta"] {
            if case .trace? = WebAgentProvider.parseSSEObject(traced(kind: kind, payload: ["delta": "not authority"])) {
                XCTFail("Retired body trace must not be admitted: \(kind)")
            }
        }

        guard case .trace(let failedTrace)? = WebAgentProvider.parseSSEObject(traced(
            kind: "run.failed",
            status: "failed",
            payload: ["reason": "host failed"]
        )) else {
            return XCTFail("Expected failure trace")
        }
        XCTAssertEqual(failedTrace.payload.reason, "host failed")

        guard case .trace(let completedTrace)? = WebAgentProvider.parseSSEObject(traced(
            kind: "run.completed",
            status: "succeeded"
        )) else {
            return XCTFail("Expected completion trace")
        }
        XCTAssertEqual(completedTrace.kind, .runCompleted)
    }

    func testMaxIterationsMessageIsNonFatal() throws {
        XCTAssertTrue(
            WebAgentProvider.isNonFatalAgentErrorMessage(
                "Reached max iterations (8) without final answer."
            )
        )
        XCTAssertFalse(
            WebAgentProvider.isNonFatalAgentErrorMessage(
                "upstream request failed: 401 unauthorized"
            )
        )
    }

    func testSSEFramePayloadSupportsNoSpaceAndMultilineData() throws {
        let json = try traceJSON(kind: "phase.changed", payload: ["phase": "processing"], pretty: true)
        let payload = WebAgentProvider.sseFramePayload(
            from: json.components(separatedBy: "\n")
                .map { "data:\($0)" }
                .compactMap(WebAgentProvider.sseDataLineValue)
        )

        guard case .trace(let trace)? = try WebAgentProvider.parseSSEPayload(payload ?? "", token: "token") else {
            return XCTFail("Expected assistant trace")
        }
        XCTAssertEqual(trace.payload.phase, "processing")
    }

    func testSSEFramePayloadIgnoresNonDataLines() throws {
        let json = try traceJSON(kind: "run.completed", status: "succeeded")
        let payload = WebAgentProvider.sseFramePayload(from: [
            ": keep-alive",
            "event: message",
            "data: \(json)"
        ].compactMap(WebAgentProvider.sseDataLineValue))

        XCTAssertEqual(payload, json)
        guard case .trace(let trace)? = try WebAgentProvider.parseSSEPayload(payload ?? "", token: "token") else {
            return XCTFail("Expected completion trace")
        }
        XCTAssertEqual(trace.kind, .runCompleted)
    }

    func testParseEncryptedSSEPayload() throws {
        let token = "mobile-access-token"
        var encrypted = try WebAgentProvider.encryptMobileJSON(
            traced(kind: "run.failed", status: "failed", payload: ["reason": "host failed"]),
            token: token
        )
        encrypted["type"] = "encrypted"
        let data = try JSONSerialization.data(withJSONObject: encrypted, options: [.prettyPrinted])
        let payload = try XCTUnwrap(String(data: data, encoding: .utf8))

        guard case .trace(let trace)? = try WebAgentProvider.parseSSEPayload(payload, token: token) else {
            return XCTFail("Expected encrypted failure trace")
        }
        XCTAssertEqual(trace.payload.reason, "host failed")
    }

    func testHostAgentPayloadLeavesLLMConfigOnHost() throws {
        let input = ModelInput(
            dateRange: .init(start: "2026-01-01T00:00:00Z", end: "2026-01-02T00:00:00Z"),
            candidates: [],
            userPrefs: ["message": "用浏览器打开百度搜索 opencode"]
        )

        let payload = WebAgentProvider.makeHostAgentPayload(input: input)
        XCTAssertNil(payload["model"])
        XCTAssertNil(payload["customEndpoint"])
        XCTAssertNil(payload["customApiKey"])
        XCTAssertEqual(payload["allow_local_dev_tools"] as? Bool, true)
        XCTAssertEqual(payload["max_iterations"] as? Int, 0)
        XCTAssertFalse((payload["language"] as? String ?? "").isEmpty)

        let messages = try XCTUnwrap(payload["messages"] as? [[String: String]])
        XCTAssertEqual(messages.first?["role"], "user")
        XCTAssertEqual(messages.first?["content"], "用浏览器打开百度搜索 opencode")
    }

    func testHostPayloadCarriesObservedParentButNeverDeviceConfigurationRevision() {
        let input = ModelInput(dateRange: .init(start: "2026-01-01", end: "2026-01-02"), candidates: [],
            userPrefs: ["message": "Follow up", "expected_parent_message_id": "committed-host-parent",
                        "expected_configuration_revision": "999"])
        let payload = WebAgentProvider.makeHostAgentPayload(input: input)
        XCTAssertEqual(payload["expected_parent_message_id"] as? String, "committed-host-parent")
        XCTAssertNil(payload["expected_configuration_revision"], "Only the authoritative Host frontier may provide its revision")
    }

    func testHostAgentPayloadUsesProvidedMaxIterations() throws {
        let input = ModelInput(
            dateRange: .init(start: "2026-01-01T00:00:00Z", end: "2026-01-02T00:00:00Z"),
            candidates: [],
            userPrefs: [
                "message": "继续",
                "web_agent_max_iterations": "42",
            ]
        )

        let payload = WebAgentProvider.makeHostAgentPayload(input: input)
        XCTAssertEqual(payload["max_iterations"] as? Int, 42)
    }

    func testHostAgentPayloadForwardsAgentModeWithoutLLMConfig() throws {
        let input = ModelInput(
            dateRange: .init(start: "2026-01-01T00:00:00Z", end: "2026-01-02T00:00:00Z"),
            candidates: [],
            userPrefs: [
                "message": "分析一个复杂问题",
                "agent_mode": "true",
            ]
        )

        let payload = WebAgentProvider.makeHostAgentPayload(input: input)
        XCTAssertEqual(payload["agent_mode"] as? Bool, true)
        XCTAssertNil(payload["model"])
        XCTAssertNil(payload["customEndpoint"])
        XCTAssertNil(payload["customApiKey"])
    }

    func testHostAgentPayloadForwardsResponseStyle() throws {
        let input = ModelInput(
            dateRange: .init(start: "2026-01-01T00:00:00Z", end: "2026-01-02T00:00:00Z"),
            candidates: [],
            userPrefs: [
                "message": "帮我分析",
                "response_style": "professional",
            ]
        )

        let payload = WebAgentProvider.makeHostAgentPayload(input: input)
        XCTAssertEqual(payload["response_style"] as? String, "professional")
        XCTAssertNil(payload["model"])
        XCTAssertNil(payload["customEndpoint"])
        XCTAssertNil(payload["customApiKey"])
    }

    func testHostAgentPayloadForwardsPreferredNativeAppHostId() throws {
        let input = ModelInput(
            dateRange: .init(start: "2026-01-01T00:00:00Z", end: "2026-01-02T00:00:00Z"),
            candidates: [],
            userPrefs: [
                "message": "观察宿主机桌面",
                "native_app_host_id": "desktop-host",
            ]
        )

        let payload = WebAgentProvider.makeHostAgentPayload(input: input)
        XCTAssertEqual(payload["native_app_host_id"] as? String, "desktop-host")
    }

    func testHostAgentPayloadForwardsStructuredMessagesAndConversationId() throws {
        let messages = #"""
        [
          {"role":"user","content":"打开系统设置"},
          {"role":"assistant","content":"已经打开。"},
          {"role":"user","content":"继续点击隐私与安全性"}
        ]
        """#
        let input = ModelInput(
            dateRange: .init(start: "2026-01-01T00:00:00Z", end: "2026-01-02T00:00:00Z"),
            candidates: [],
            userPrefs: [
                "message": "fallback",
                "host_agent_messages": messages,
                "host_conversation_id": "conversation-1",
            ]
        )

        let payload = WebAgentProvider.makeHostAgentPayload(input: input)
        let forwarded = try XCTUnwrap(payload["messages"] as? [[String: Any]])
        XCTAssertEqual(payload["conversationId"] as? String, "conversation-1")
        XCTAssertEqual(forwarded.count, 3)
        XCTAssertEqual(forwarded[0]["role"] as? String, "user")
        XCTAssertEqual(forwarded[1]["role"] as? String, "assistant")
        XCTAssertEqual(forwarded[2]["content"] as? String, "继续点击隐私与安全性")
    }

    func testHostAgentPayloadForwardsCanonicalTurnIdentityAtomically() throws {
        let input = ModelInput(
            dateRange: .init(start: "2026-01-01T00:00:00Z", end: "2026-01-02T00:00:00Z"),
            candidates: [],
            userPrefs: [
                "message": "Continue",
                "host_conversation_id": "conversation-1",
                "submission_id": "submission-1",
                "user_message_id": "user-1",
                "assistant_message_id": "assistant-1",
            ]
        )

        let payload = WebAgentProvider.makeHostAgentPayload(input: input)

        XCTAssertEqual(payload["conversationId"] as? String, "conversation-1")
        XCTAssertEqual(payload["submission_id"] as? String, "submission-1")
        XCTAssertEqual(payload["user_message_id"] as? String, "user-1")
        XCTAssertEqual(payload["assistant_message_id"] as? String, "assistant-1")
    }

    func testHostAgentPayloadDoesNotEmbedPermissionDecisions() throws {
        let decisions = #"""
        [
          {"tool_name":"disk_usage","match":"broad:/","decision":"allow"}
        ]
        """#
        let input = ModelInput(
            dateRange: .init(start: "2026-01-01T00:00:00Z", end: "2026-01-02T00:00:00Z"),
            candidates: [],
            userPrefs: [
                "message": "继续",
                "host_permission_decisions": decisions,
            ]
        )

        let payload = WebAgentProvider.makeHostAgentPayload(input: input)
        XCTAssertNil(payload["permission_decisions"])
    }

    func testConfiguredHostAgentPayloadBindsHostAndMobileClientIdentity() throws {
        let baseURL = try XCTUnwrap(URL(string: "https://host.example"))
        let provider = WebAgentProvider(config: .init(
            baseURL: baseURL,
            token: "token",
            expectedHostId: "cli-host-paired-1234",
            clientId: "ios-client-1234"
        ))
        let input = ModelInput(
            dateRange: .init(start: "2026-01-01T00:00:00Z", end: "2026-01-02T00:00:00Z"),
            candidates: [],
            userPrefs: ["message": "继续"]
        )

        let payload = provider.makeHostAgentPayload(input: input)

        XCTAssertEqual(payload["expected_host_id"] as? String, "cli-host-paired-1234")
        XCTAssertEqual(payload["client_id"] as? String, "ios-client-1234")
    }

    func testConfiguredHostAgentPayloadAdvertisesExplicitHybridDeviceTools() throws {
        let baseURL = try XCTUnwrap(URL(string: "https://host.example"))
        let provider = WebAgentProvider(config: .init(
            baseURL: baseURL,
            token: "token",
            expectedHostId: "cli-host-paired-1234",
            clientId: "ios-client-1234"
        ))
        let input = ModelInput(
            dateRange: .init(start: "2026-01-01T00:00:00Z", end: "2026-01-02T00:00:00Z"),
            candidates: [],
            userPrefs: [
                "message": "Use my calendar",
                "host_conversation_id": "conversation-1",
                "hybrid_device_tools_enabled": "true",
                "hybrid_device_result_egress_enabled": "true",
                "hybrid_device_capabilities": "files_read,files_write,clipboard,location,calendar",
            ]
        )

        let payload = provider.makeHostAgentPayload(input: input)
        let bridge = try XCTUnwrap(payload["device_bridge"] as? [String: Any])
        let tools = try XCTUnwrap(bridge["tools"] as? [[String: Any]])

        XCTAssertEqual(bridge["host_id"] as? String, "cli-host-paired-1234")
        XCTAssertEqual(bridge["client_id"] as? String, "ios-client-1234")
        XCTAssertEqual(bridge["session_id"] as? String, "conversation-1")
        XCTAssertFalse(tools.isEmpty)
        XCTAssertEqual(bridge["allow_result_egress"] as? Bool, true)
        XCTAssertTrue(tools.allSatisfy {
            $0["allow_result_egress"] as? Bool == bridge["allow_result_egress"] as? Bool
        })
    }

    func testParseDeviceToolRequest() throws {
        let event = WebAgentProvider.parseSSEObject(traced([
            "type": "device_tool_request",
            "protocol_version": 1,
            "id": "call-device-1",
            "nonce": "nonce-device-1",
            "host_id": "cli-host-paired-1234",
            "client_id": "ios-client-1234",
            "session_id": "conversation-1",
            "tool_name": "device_get_location",
            "capability": "location",
            "arguments": [:],
            "args": [:],
            "args_digest": "digest-1",
            "expires_at": 2_000_000_000,
        ], kind: "tool.permission.requested", status: "waiting", spanID: "call-device-1", payload: [
            "tool_name": "device_get_location",
            "call_id": "call-device-1",
            "permission_request_id": "call-device-1",
            "permission_scope": "location",
        ]))

        guard case .deviceToolRequest(let request, _)? = event else {
            return XCTFail("Expected device_tool_request event")
        }
        XCTAssertEqual(request.toolName, "device_get_location")
        XCTAssertEqual(request.capability, .location)
        XCTAssertEqual(request.protocolVersion, 1)
    }

    func testHybridAdvertisementIncludesOnlySelectedCapabilities() throws {
        let baseURL = try XCTUnwrap(URL(string: "https://host.example"))
        let provider = WebAgentProvider(config: .init(
            baseURL: baseURL,
            token: "token",
            expectedHostId: "cli-host-paired-1234",
            clientId: "ios-client-1234"
        ))
        func payload(capabilities: String) -> [String: Any] {
            provider.makeHostAgentPayload(input: ModelInput(
                dateRange: .init(start: "2026-01-01T00:00:00Z", end: "2026-01-02T00:00:00Z"),
                candidates: [],
                userPrefs: [
                    "message": "Use this device",
                    "host_conversation_id": "conversation-1",
                    "hybrid_device_tools_enabled": "true",
                    "hybrid_device_result_egress_enabled": "true",
                    "hybrid_device_capabilities": capabilities,
                ]
            ))
        }

        XCTAssertNil(payload(capabilities: "")["device_bridge"])
        let bridge = try XCTUnwrap(payload(capabilities: "location")["device_bridge"] as? [String: Any])
        let tools = try XCTUnwrap(bridge["tools"] as? [[String: Any]])
        XCTAssertEqual(tools.count, 1)
        let schema = try XCTUnwrap(tools.first?["schema"] as? [String: Any])
        let function = try XCTUnwrap(schema["function"] as? [String: Any])
        XCTAssertEqual(function["name"] as? String, "device_get_location")
    }

    func testMobileURLNormalizesCopiedMobileEndpoint() throws {
        let base = try XCTUnwrap(URL(string: "http://192.168.1.8:5001/api/mobile/login"))
        let url = WebAgentProvider.mobileURL(baseURL: base, path: "chat/agent/v2/stream")

        XCTAssertEqual(
            url.absoluteString,
            "http://192.168.1.8:5001/api/mobile/chat/agent/v2/stream"
        )
    }

    func testControlPlaneURLUsesHostOriginInsteadOfAPIPath() throws {
        let base = try XCTUnwrap(URL(string: "https://host.example:7443/api/mobile/agent/settings"))
        XCTAssertEqual(
            WebAgentProvider.controlPlaneURL(baseURL: base, path: "/settings#channels")?.absoluteString,
            "https://host.example:7443/settings#channels"
        )
        XCTAssertEqual(
            WebAgentProvider.controlPlaneURL(baseURL: base, path: "settings/channels?tab=active")?.absoluteString,
            "https://host.example:7443/settings/channels?tab=active"
        )
    }

    func testRejectsLegacyUntracedContextUsageEvent() throws {
        let event = WebAgentProvider.parseSSEObject([
            "type": "context_usage",
            "used_tokens": 4_096,
            "limit_tokens": 65_536,
            "context_window_tokens": 65_536,
        ])

        guard case .error(let message)? = event else {
            return XCTFail("Expected a protocol violation")
        }
        XCTAssertTrue(message.contains("malformed context usage projection"))
    }

    func testParsesScopedV2ContextUsageWithoutTrace() throws {
        let event = WebAgentProvider.parseSSEObject([
            "protocol_version": 2,
            "type": "context_usage",
            "session_id": "session-1",
            "source_host_id": "host-1",
            "used_tokens": 4_096,
            "context_window_tokens": 65_536,
            "prompt_token_limit": 61_440,
            "reserved_output_tokens": 4_096,
            "message_tokens": 3_000,
            "tool_schema_tokens": 600,
            "component_tokens": 200,
            "tool_result_tokens": 100,
            "conversation_tokens": 2_900,
            "message_count": 8,
            "tool_count": 12,
            "usage_percent": 6.25,
        ])

        guard case .contextUsageUpdated(let usage)? = event else {
            return XCTFail("Expected scoped context usage projection")
        }
        XCTAssertEqual(usage.sessionID, "session-1")
        XCTAssertEqual(usage.usedTokens, 4_096)
        XCTAssertEqual(usage.promptTokenLimit, 61_440)
    }

    func testRemovedFullSessionProjectionCannotSupplyConversationState() throws {
        let cursor: UInt64 = 675_868_731_199_239_600
        let event = WebAgentProvider.parseSSEObject([
            "protocol_version": 2,
            "type": "core_session_projection_updated",
            "session_id": "session-1",
            "source_host_id": "host-1",
            "cursor": cursor,
            "revision": cursor,
            "snapshot": [
                "session_id": "session-1",
                "revision": cursor,
            ],
        ])

        guard case .error? = event else {
            return XCTFail("Removed full session packets must not be accepted as state")
        }
    }

    func testRemoteHostRejectsNestedCoreLifecycleEnvelope() throws {
        let event = WebAgentProvider.parseSSEObject([
            "protocol_version": 2,
            "session_id": "session-1",
            "cursor": ["sequence": 1],
            "revision": 1,
            "event": [
                "type": "part_updated",
                "turn_id": "turn-1",
                "projection_revision": 1,
                "update": ["type": "created", "part_id": "part-1"],
                "part": ["kind": "text", "id": "part-1", "text": "old wire"],
            ],
        ])

        guard case .error(let message)? = event else {
            return XCTFail("Expected nested Host event to be rejected")
        }
        XCTAssertTrue(message.contains("missing Core Trace"))
    }

    func testParsesUntracedCoreCapabilityProjection() throws {
        let event = WebAgentProvider.parseSSEObject([
            "protocol_version": 2,
            "type": "core_capabilities_activated",
            "session_id": "session-1",
            "turn_id": "turn-1",
            "cursor": 3,
            "revision": 4,
            "source": "skill:documents",
            "capability_ids": ["document.create"],
        ])

        guard case .coreCapabilitiesActivated(let activation)? = event else {
            return XCTFail("Expected untraced Core capability projection")
        }
        XCTAssertEqual(activation.sessionID, "session-1")
        XCTAssertEqual(activation.capabilityIDs, ["document.create"])
    }

    func testParsesUntracedCorePermissionProjection() throws {
        let event = WebAgentProvider.parseSSEObject([
            "protocol_version": 2,
            "type": "core_permission_projection_updated",
            "session_id": "session-1",
            "turn_id": "turn-1",
            "permission_projection_revision": 3,
            "permission_projection": [
                "revision": 3,
                "pending": [],
            ],
        ])

        guard case .corePermissionProjectionUpdated(let projection)? = event else {
            return XCTFail("Expected untraced Core permission projection")
        }
        XCTAssertEqual(projection.revision, 3)
        XCTAssertTrue(projection.pending.isEmpty)
    }

    func testParsesUntracedCoreMemoryRuntimeProjection() throws {
        let event = WebAgentProvider.parseSSEObject([
            "protocol_version": 2,
            "type": "core_memory_runtime_projection_updated",
            "session_id": "session-1",
            "turn_id": "turn-1",
            "memory_runtime_revision": 4,
            "memory_runtime_projection": [
                "revision": 4,
                "jobs": [],
            ],
        ])

        guard case .coreMemoryRuntimeProjectionUpdated(let projection)? = event else {
            return XCTFail("Expected untraced Core memory runtime projection")
        }
        guard case .object(let object) = projection else {
            return XCTFail("Expected object memory projection")
        }
        XCTAssertEqual(object["revision"], .number(4))
    }

    func testRejectsMalformedLegacyContextUsageAsMalformedProjection() throws {
        let event = WebAgentProvider.parseSSEObject([
            "type": "context_usage",
            "used_tokens": 4_096,
        ])

        guard case .error(let message)? = event else {
            return XCTFail("Expected legacy protocol error")
        }
        XCTAssertTrue(message.contains("malformed context usage projection"))
    }

    func testResolveToolURLRewritesLoopbackAgainstLanHost() throws {
        let base = try XCTUnwrap(URL(string: "http://192.168.1.8:5001/api/mobile/login"))

        XCTAssertEqual(
            WebAgentProvider.resolveToolURL(URL(string: "http://127.0.0.1:5001/static/call.png"), baseURL: base)?.absoluteString,
            "http://192.168.1.8:5001/static/call.png"
        )
        XCTAssertEqual(
            WebAgentProvider.resolveToolURL(URL(string: "http://localhost:3000/app"), baseURL: base)?.absoluteString,
            "http://192.168.1.8:3000/app"
        )
        XCTAssertEqual(
            WebAgentProvider.resolveToolURL(URL(string: "http://localhost/static/call.png"), baseURL: base)?.absoluteString,
            "http://192.168.1.8:5001/static/call.png"
        )
        XCTAssertEqual(
            WebAgentProvider.resolveToolURL(URL(string: "https://example.com/path"), baseURL: base)?.absoluteString,
            "https://example.com/path"
        )
    }

    func testResolveToolURLKeepsLoopbackWhenBaseIsLoopbackForSimulator() throws {
        let base = try XCTUnwrap(URL(string: "http://127.0.0.1:5001/api/mobile/login"))

        XCTAssertEqual(
            WebAgentProvider.resolveToolURL(URL(string: "http://localhost:3000/app"), baseURL: base)?.absoluteString,
            "http://localhost:3000/app"
        )
    }

    func testMobileBridgeEncryptionRoundtrip() throws {
        let payload: [String: Any] = [
            "messages": [["role": "user", "content": "用浏览器打开百度搜索 opencode"]],
            "allow_local_dev_tools": true,
        ]

        let encrypted = try WebAgentProvider.encryptMobileJSON(payload, token: "mobile-access-token")

        XCTAssertEqual(encrypted["alg"] as? String, "AES-256-GCM")
        XCTAssertNil(encrypted["messages"])

        let decrypted = try WebAgentProvider.decryptMobileJSON(encrypted, token: "mobile-access-token")
        let messages = try XCTUnwrap(decrypted["messages"] as? [[String: String]])
        XCTAssertEqual(messages.first?["content"], "用浏览器打开百度搜索 opencode")
        XCTAssertEqual(decrypted["allow_local_dev_tools"] as? Bool, true)
    }

    func testCertificateFingerprintNormalization() throws {
        let raw = "aa:bb cc-dd-ee-ff-00-11-22-33-44-55-66-77-88-99-aa-bb-cc-dd-ee-ff-00-11-22-33-44-55-66-77-88-99"
        let normalized = WebAgentProvider.Config.normalizedFingerprint(raw)

        XCTAssertEqual(normalized, "AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899")
        XCTAssertNil(WebAgentProvider.Config.normalizedFingerprint("not-a-sha256"))

        let provided = "08:70:FB:7C:20:B7:53:4B:E8:C8:F7:B0:52:2B:C6:A9:80:4C:67:48:74:93:2B:61:70:97:C6:5B:C8:48:AB:0C"
        XCTAssertEqual(
            WebAgentProvider.Config.normalizedFingerprint(provided),
            "0870FB7C20B7534BE8C8F7B0522BC6A9804C674874932B617097C65BC848AB0C"
        )
    }

    private func traced(
        _ object: [String: Any] = [:],
        kind: String,
        status: String = "running",
        spanID: String? = nil,
        payload: [String: Any] = [:],
        sequence: UInt64 = 1
    ) -> [String: Any] {
        var result = object
        var trace: [String: Any] = [
            "schema_version": agentTraceSchemaVersion,
            "trace_id": "trace-web-provider-tests",
            "run_id": "run-web-provider-tests",
            "conversation_id": "conversation-web-provider-tests",
            "turn_id": "turn-web-provider-tests",
            "event_id": "event-\(sequence)",
            "sequence": sequence,
            "emitted_at": "2026-07-19T00:00:00Z",
            "kind": kind,
            "status": status,
            "visibility": "summary",
            "payload": payload,
        ]
        if let spanID {
            trace["span_id"] = spanID
        }
        result["trace"] = trace
        return result
    }

    private func traceJSON(
        kind: String,
        status: String = "running",
        payload: [String: Any] = [:],
        pretty: Bool = false
    ) throws -> String {
        let data = try JSONSerialization.data(
            withJSONObject: traced(kind: kind, status: status, payload: payload),
            options: pretty ? [.prettyPrinted, .sortedKeys] : [.sortedKeys]
        )
        return try XCTUnwrap(String(data: data, encoding: .utf8))
    }
}


extension WebAgentProviderTests {
    func testHostPermissionWireVersionFourAndHardCutRejectsOldIdentityAndSSE() throws {
        // Literal 4 is the Rust HOST_PERMISSION_PROTOCOL_VERSION contract, not a
        // fixture derived from the Swift value under test.
        XCTAssertEqual(WebAgentProvider.hostPermissionProtocolVersion, 4)
        let identity = Data(#"{"host_identity":{"id":"rust-host","permission_protocol_version":4}}"#.utf8)
        let settings = try JSONDecoder().decode(WebAgentProvider.HostAgentSettings.self, from: identity)
        XCTAssertEqual(settings.host_identity?.permission_protocol_version, 4)
        for old in [1, 2, 3] {
            let data = Data("{\"host_identity\":{\"id\":\"rust-host\",\"permission_protocol_version\":\(old)}}".utf8)
            XCTAssertThrowsError(try JSONDecoder().decode(WebAgentProvider.HostAgentSettings.self, from: data))
        }
        func event(version: Any?) throws -> WebAgentStreamEvent? {
            var object: [String: Any] = ["type": "permission_request", "allow_conversation": true, "id": "request-3", "tool_name": "write_file", "args": ["path": "/original"], "nonce": "nonce-3"]
            object["protocol_version"] = version
            let wire = traced(object, kind: "tool.permission.requested", status: "waiting", spanID: "call-3", payload: ["tool_name": "write_file", "call_id": "call-3", "permission_request_id": "request-3"])
            let data = try JSONSerialization.data(withJSONObject: wire)
            return try WebAgentProvider.parseSSEPayload(String(decoding: data, as: UTF8.self), token: "test")
        }
        guard case .permissionRequest(let request, _)? = try event(version: 4) else { return XCTFail("Expected Rust v4 permission request") }
        XCTAssertEqual(request.protocolVersion, 4)
        XCTAssertEqual(request.id, "request-3")
        for version: Any? in [nil, 1, 2, 3, "4"] {
            guard case .error? = try event(version: version) else { return XCTFail("Unsupported permission wire must fail closed: \(String(describing: version))") }
        }
    }
}
