// SPDX-License-Identifier: Apache-2.0

import Foundation
import Testing
@testable import Nola

struct MacLocalHostPromptBoundaryTests {
    @Test func localRunPreservesTypedContextInsteadOfPromotingItToHostFact() throws {
        let components = [
            MacCoreContextComponent(
                kind: "memory",
                id: "memory-1",
                content: "Untrusted recalled context"
            ),
            MacCoreContextComponent(
                kind: "prompt_template",
                id: "template-1",
                content: "User-selected instruction"
            ),
        ]
        let request = try MacLocalAgentRunRequest(
            conversationID: "prompt-boundary",
            prompt: "Hello",
            contextComponents: components,
            projectURL: FileManager.default.temporaryDirectory,
            attachmentURLs: [],
        )

        #expect(request.contextComponents == components)
        #expect(request.contextComponents.map(\.kind) == ["memory", "prompt_template"])
        #expect(!request.contextComponents.contains { $0.kind == "system" })
    }

    @Test func remoteRunPreservesTypedContextInsteadOfFlatteningItIntoSystemText() throws {
        let components = [
            MacCoreContextComponent(
                kind: "memory",
                id: "memory-remote",
                content: "Recalled context"
            ),
            MacCoreContextComponent(
                kind: "attachment",
                id: "attachment-remote",
                content: "Document contents"
            ),
        ]

        let payload = try MacRemoteContextPayload.encode(components)

        #expect(payload.count == 2)
        #expect(payload.compactMap { $0["kind"] as? String } == ["memory", "attachment"])
        #expect(!payload.contains { ($0["kind"] as? String) == "system" })
    }
}

@MainActor
struct MacPlatformContextAuthorizationTests {
    @Test func platformContextPreservesAuthorizationAndObservation() {
        let observed = Date(timeIntervalSince1970: 1_788_640_800)
        let item = MacPlatformDataContext(sourceID: "contacts", title: "Contacts", content: "Fixture", observedAt: observed, authorization: .available)
        let component = item.coreComponent
        #expect(component.source?.authorization == "available")
        #expect(component.source?.observedAt == ISO8601DateFormatter().string(from: observed))
        #expect(component.content == .string("Fixture"))
    }

    @Test func unspecifiedPlatformAuthorizationIsNotPromotedToAvailable() {
        let item = MacPlatformDataContext(sourceID: "contacts", title: "Contacts", content: "Fixture", observedAt: nil)
        #expect(item.coreComponent.source?.authorization == "notDetermined")
    }
}
