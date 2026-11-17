// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS permission envelope, presentation identity, custom instruction composer, and guidance suite.

import AppKit
import SwiftUI
import XCTest
@testable import Nola

final class MacPermissionSuiteTests: XCTestCase {
    private func makeCustomRequest(_ id: String = "pending", records: [MacPermissionInstruction] = []) -> MacRemotePermissionRequest {
        MacRemotePermissionRequest(
            id: id, allowConversation: true, protocolVersion: 1, nonce: "nonce-\(id)", hostID: "host", clientID: "client",
            sessionID: "conversation", toolName: "write_file", arguments: #"{"path":"/original","content":"same"}"#,
            permissionInstructions: records, reason: "Confirm", message: "Confirm", expiresAt: nil)
    }

    private func makeIdentityRequest(
        id: String = "permission", host: String = "host", client: String = "client",
        session: String = "session", nonce: String = "challenge", call: String? = "call",
        instructions: [MacPermissionInstruction] = [], reason: String = "Review",
        expiresAt: Int64? = 100
    ) -> MacRemotePermissionRequest {
        MacRemotePermissionRequest(
            id: id, allowConversation: true, protocolVersion: 1, nonce: nonce,
            hostID: host, clientID: client, sessionID: session, toolName: "write_file",
            arguments: #"{"path":"/workspace/example"}"#,
            permissionInstructions: instructions, callID: call, reason: reason,
            message: "Permission needed", expiresAt: expiresAt)
    }

    func testPermissionEnvelopeEligibilityReaskedGuidanceAndMetadataValidation() throws {
        var object: [String: Any] = [
            "type": "permission_request", "protocol_version": 1,
            "allow_conversation": false, "request_id": "opaque", "nonce": "nonce", "host_id": "host",
            "client_id": "client", "session_id": "conversation", "tool_name": "opaque-executor", "args": [:]
        ]
        func decode() throws -> MacLocalAgentEvent? {
            MacAgentHostRuntime.decode(
                line: String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self),
                conversationID: "conversation",
                controlSessionID: "conversation"
            )
        }
        XCTAssertFalse(try XCTUnwrap(decode()?.permissionRequest).allowConversation)
        object.removeValue(forKey: "allow_conversation")
        XCTAssertNil(try decode()?.permissionRequest, "Missing eligibility must not become a reusable offer")
        object["allow_conversation"] = false
        object["protocol_version"] = 3
        XCTAssertNil(try decode()?.permissionRequest, "Old permission protocol must fail closed")

        let reaskedObject: [String: Any] = [
            "type": "permission_request", "allow_conversation": true, "protocol_version": 1, "request_id": "reasked", "nonce": "new-nonce",
            "host_id": "host", "client_id": "client", "session_id": "conversation",
            "tool_name": "write_file", "args": ["path": "/original"],
            "permission_instructions": [["permission_request_id": "original", "instruction": "Use a preview <first>."]]
        ]
        let line = String(data: try JSONSerialization.data(withJSONObject: reaskedObject), encoding: .utf8)!
        let event = try XCTUnwrap(MacAgentHostRuntime.decode(line: line, conversationID: "conversation", controlSessionID: "conversation"))
        let reasked = try XCTUnwrap(event.permissionRequest)
        XCTAssertEqual(reasked.id, "reasked")
        XCTAssertEqual(reasked.permissionInstructions.first?.permissionRequestID, "original")
        XCTAssertEqual(reasked.permissionInstructions.first?.instruction, "Use a preview <first>.")
        XCTAssertTrue(reasked.arguments.contains("/original"))
        XCTAssertFalse(reasked.arguments.contains("preview"))
        XCTAssertEqual(reasked.decision(allow: true).payload["resolution"] as? [String: String], ["type": "authorization", "decision": "allow_once"])

        XCTAssertEqual(MacPermissionInstruction.decode(nil), [])
        for value: Any in [
            NSNull(),
            "text",
            [["permission_request_id": "old", "instruction": 1]],
            [["permission_request_id": "old", "instruction": " "]],
            [["permission_request_id": "old", "instruction": "text", "grant": true]]
        ] {
            XCTAssertNil(MacPermissionInstruction.decode(value))
        }
    }

    func testPermissionPresentationIdentityPreservesDecisionAndIsolatesSuccessors() {
        let original = makeIdentityRequest()
        let updated = makeIdentityRequest(
            instructions: [MacPermissionInstruction(permissionRequestID: "permission", instruction: "Preview first")],
            reason: "Updated explanation",
            expiresAt: 200
        )
        XCTAssertNotEqual(original, updated, "Authorization still compares complete envelopes.")
        XCTAssertEqual(original.presentationIdentity, updated.presentationIdentity)

        let identity = original.presentationIdentity
        let successors = [
            makeIdentityRequest(id: "next"),
            makeIdentityRequest(host: "other-host"),
            makeIdentityRequest(client: "other-client"),
            makeIdentityRequest(session: "other-session"),
            makeIdentityRequest(nonce: "new-challenge"),
            makeIdentityRequest(call: "other-call"),
            makeIdentityRequest(call: nil)
        ]
        for successor in successors {
            XCTAssertNotEqual(identity, successor.presentationIdentity)
        }
        XCTAssertEqual(Set(successors.map(\.presentationIdentity)).count, successors.count)

        let decision = original.decision(allow: true)
        XCTAssertEqual(decision.requestID, original.id)
        XCTAssertEqual(decision.nonce, original.nonce)
        XCTAssertEqual(decision.hostID, original.hostID)
        XCTAssertEqual(decision.clientID, original.clientID)
        XCTAssertEqual(decision.sessionID, original.sessionID)
        XCTAssertTrue(decision.resolution.isAllowed)
    }

    @MainActor
    func testTypedInterventionPayloadAndUTF8HistoryBudgets() throws {
        let original = makeCustomRequest()
        let decision = try original.instructionDecision("  Preview first.  ")
        let data = try JSONSerialization.data(withJSONObject: decision.payload)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let resolution = try XCTUnwrap(payload["resolution"] as? [String: Any])
        XCTAssertEqual(payload["request_id"] as? String, original.id)
        XCTAssertEqual(payload["nonce"] as? String, original.nonce)
        XCTAssertEqual(payload["session_id"] as? String, original.sessionID)
        XCTAssertEqual(resolution["type"] as? String, "intervention")
        XCTAssertEqual(resolution["intervention"] as? [String: String], ["type": "provide_instruction", "instruction": "Preview first."])
        XCTAssertNil(resolution["decision"])
        XCTAssertNil(resolution["grant"])
        XCTAssertNil(payload["input"])
        XCTAssertFalse(decision.resolution.isAllowed)
        XCTAssertEqual(original.arguments, #"{"path":"/original","content":"same"}"#)
        XCTAssertEqual(original.decision(allow: false).payload["resolution"] as? [String: String], ["type": "authorization", "decision": "deny"])

        XCTAssertThrowsError(try makeCustomRequest().instructionDecision(" \n "))
        XCTAssertNoThrow(try makeCustomRequest().instructionDecision(String(repeating: "界", count: 1365)))
        XCTAssertThrowsError(try makeCustomRequest().instructionDecision(String(repeating: "界", count: 1366)))
        let record = MacPermissionInstruction(permissionRequestID: "old", instruction: "prior")
        XCTAssertNoThrow(try makeCustomRequest(records: Array(repeating: record, count: 15)).instructionDecision("last"))
        let full = makeCustomRequest(records: Array(repeating: record, count: 16))
        XCTAssertThrowsError(try full.instructionDecision("too many"))
        XCTAssertTrue(full.decision(allow: true).resolution.isAllowed)
        XCTAssertFalse(full.decision(allow: false).resolution.isAllowed)
        let long = MacPermissionInstruction(permissionRequestID: "prior", instruction: String(repeating: "x", count: 4096))
        XCTAssertThrowsError(try makeCustomRequest(records: Array(repeating: long, count: 3)).instructionDecision(String(repeating: "x", count: 4096)))
    }

    @MainActor
    func testInstructionComposerDraftRetryAndInFlightSuccessorIsolation() async {
        let composer = MacPermissionInstructionComposer()
        composer.text = "Preview first."
        var delivered: [String] = []
        await composer.submit(request: makeCustomRequest()) { text in
            delivered.append(text)
            throw MacPermissionInstructionError.unsupported
        }
        XCTAssertEqual(composer.text, "Preview first.")
        XCTAssertNotNil(composer.error)
        XCTAssertNil(composer.submittedRequestID)
        XCTAssertFalse(composer.isSending)
        await composer.submit(request: makeCustomRequest()) { delivered.append($0) }
        XCTAssertEqual(delivered, ["Preview first.", "Preview first."])
        XCTAssertEqual(composer.submittedRequestID, "pending")
        XCTAssertEqual(composer.text, "")
        XCTAssertNil(composer.error)
        await composer.submit(request: makeCustomRequest()) { _ in XCTFail("Answered interaction cannot submit twice") }

        let old = MacPermissionInstructionComposer()
        old.text = "old guidance"
        var response: CheckedContinuation<Void, Never>?
        let sending = Task {
            await old.submit(request: self.makeCustomRequest("old")) { _ in
                await withCheckedContinuation { response = $0 }
            }
        }
        while response == nil { await Task.yield() }
        XCTAssertTrue(old.isSending)
        await old.submit(request: makeCustomRequest("old")) { _ in XCTFail("Duplicate guidance") }
        let successor = MacPermissionInstructionComposer()
        successor.text = "new draft"
        response?.resume()
        await sending.value
        XCTAssertEqual(old.submittedRequestID, "old")
        XCTAssertEqual(successor.text, "new draft")
        XCTAssertNil(successor.submittedRequestID)
        XCTAssertTrue(successor.canSubmit(request: makeCustomRequest("new")))
    }

    @MainActor
    func testPermissionFailureSettlesBusyStateExplicitRetryClearsErrorAndSettingsURLsMatch() async throws {
        XCTAssertTrue(MacDataSourceSnapshot.Kind.contacts.permissionSettingsURL.hasSuffix("Privacy_Contacts"))
        XCTAssertTrue(MacDataSourceSnapshot.Kind.calendar.permissionSettingsURL.hasSuffix("Privacy_Calendars"))
        XCTAssertTrue(MacDataSourceSnapshot.Kind.reminders.permissionSettingsURL.hasSuffix("Privacy_Reminders"))
        XCTAssertTrue(MacDataSourceSnapshot.Kind.notifications.permissionSettingsURL.contains("Notifications-Settings"))

        let session = try MacHostedAccessibilitySession()
        defer { session.restore() }
        let suite = "permission-guidance-\(UUID())"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { preferences.removePersistentDomain(forName: suite) }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(suite)
        defer { try? FileManager.default.removeItem(at: root) }
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        var calls = 0
        var reply: CheckedContinuation<Void, Error>?
        let service = MacPlatformDataService(
            registry: registry, startsPeriodicRefresh: false, preferences: preferences,
            permissionRequest: { _ in
                calls += 1
                if calls == 1 { try await withCheckedThrowingContinuation { reply = $0 } }
            }
        )
        let first = Task { await service.requestAccess(for: .contacts) }
        while reply == nil { await Task.yield() }
        XCTAssertTrue(service.requestingAccess.contains(.contacts))
        await service.requestAccess(for: .contacts)
        XCTAssertEqual(calls, 1, "Repeated clicks cannot launch a second system request")
        reply?.resume(throwing: NSError(domain: "PermissionFixture", code: 1,
            userInfo: [NSLocalizedDescriptionKey: "Fixture system request unavailable"]))
        await first.value
        XCTAssertTrue(service.requestingAccess.isEmpty)
        XCTAssertEqual(service.permissionRequestErrors[.contacts], "Fixture system request unavailable")
        XCTAssertNil(service.permissionRequestErrors[.calendar])
        let web = MacWebSourceStore(storeURL: root.appendingPathComponent("web.json"), registry: registry)
        let host = NSHostingView(rootView: MacDataCenterView(service: service, registry: registry,
            webSources: web, initialSection: "sources").environment(\.accessibilityEnabled, true)
            .frame(width: 680, height: 900))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 680, height: 900),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = host; window.orderFrontRegardless()
        defer { window.close() }
        try await Task.sleep(for: .milliseconds(200)); host.layoutSubtreeIfNeeded()
        XCTAssertNotNil(accessibilityObjects(host).first {
            ($0.accessibilityIdentifier?() ?? nil) == "data-source-permission-error-contacts"
        }, "A failed system request must have visible, accessible recovery feedback")
        await service.requestAccess(for: .contacts)
        XCTAssertEqual(calls, 2)
        XCTAssertNil(service.permissionRequestErrors[.contacts])
        XCTAssertTrue(service.requestingAccess.isEmpty)
    }

    @MainActor
    func testFirstRunGuideNavigatesWithoutPretendSelectionsOrPreferenceWrites() async throws {
        let session = try MacHostedAccessibilitySession()
        defer { session.restore() }
        let suite = "onboarding-guidance-\(UUID())"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { preferences.removePersistentDomain(forName: suite) }
        preferences.set("existing-host", forKey: "nola.mac.runtime.preference")
        preferences.set("existing-sources", forKey: "nola.mac.planned.sources")
        for width: CGFloat in [520, 1040] {
            for dark in [false, true] {
                var completions = 0
                let host = NSHostingView(rootView: MacOnboardingView { completions += 1 }
                    .defaultAppStorage(preferences)
                    .environment(\.colorScheme, dark ? .dark : .light)
                    .environment(\.accessibilityEnabled, true)
                    .transaction { $0.animation = nil; $0.disablesAnimations = true }
                    .frame(width: width, height: 820))
                let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 820),
                    styleMask: [.borderless], backing: .buffered, defer: false)
                window.isReleasedWhenClosed = false; window.contentView = host; window.orderFrontRegardless()
                defer { window.close() }
                for step in 0..<4 {
                    try await Task.sleep(for: .milliseconds(80))
                    host.layoutSubtreeIfNeeded()
                    let objects = accessibilityObjects(host)
                    XCTAssertFalse(objects.contains { object in
                        let role = object.accessibilityRole?() ?? nil
                        return role == .radioButton || role == .checkBox
                    }, "The guide must not offer choices that do not configure execution")
                    if let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
                        host.cacheDisplay(in: host.bounds, to: bitmap)
                        let attachment = XCTAttachment(data: try XCTUnwrap(bitmap.representation(using: .png, properties: [:])), uniformTypeIdentifier: "public.png")
                        attachment.name = "onboarding-\(step)-\(Int(width))-\(dark ? "dark" : "light")"
                        attachment.lifetime = .keepAlways; add(attachment)
                    }
                    let candidate: AnyObject? = objects.first { ($0.accessibilityIdentifier?() ?? nil) == "onboarding.next" }
                    let next: AnyObject = try XCTUnwrap(candidate)
                    XCTAssertTrue(next.accessibilityPerformPress?() ?? false)
                }
                try await Task.sleep(for: .milliseconds(60))
                XCTAssertEqual(completions, 1)
                XCTAssertEqual(preferences.string(forKey: "nola.mac.runtime.preference"), "existing-host")
                XCTAssertEqual(preferences.string(forKey: "nola.mac.planned.sources"), "existing-sources")
                window.close()
            }
        }
    }

    @MainActor
    private func accessibilityObjects(_ root: AnyObject) -> [AnyObject] {
        var queue = [root], visited = Set<ObjectIdentifier>(), result: [AnyObject] = []
        var index = 0
        while index < queue.count, result.count < 4000 {
            let object = queue[index]; index += 1
            guard visited.insert(ObjectIdentifier(object)).inserted else { continue }
            result.append(object)
            let children = object.accessibilityChildren?() ?? []
            queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
            if let view = object as? NSView { queue.append(contentsOf: view.subviews) }
        }
        return result
    }
}
