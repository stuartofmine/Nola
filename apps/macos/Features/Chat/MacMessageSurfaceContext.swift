// SPDX-License-Identifier: Apache-2.0
import SwiftUI

/// Presentation scope only. Every operation still forwards the complete Core
/// identity to its original Host; this value grants no tools or UI authority.
nonisolated struct MacMessageSurfaceScope: Hashable {
    let conversationID: String
    let messageID: String
    let runID: String?
    func contains(_ identity: AppleUISurfaceIdentity) -> Bool {
        identity.owner.conversation_id == conversationID && identity.owner.message_id == messageID
            && (runID == nil || identity.owner.originating_run_id == runID)
    }
}

extension MacUISurfaceContext {
    func scoped(to scope: MacMessageSurfaceScope, isCurrent: @escaping () -> Bool) -> Self {
        func validate(_ identity: AppleUISurfaceIdentity) throws {
            guard isCurrent(), scope.contains(identity) else { throw MacApplicationPortError.invalidResponse }
        }
        return .init(readChanges: { identity, revision in
            try validate(identity)
            let value = try await self.readChanges(identity, revision)
            try validate(identity)
            return value
        }, preview: { input in
            try validate(input.identity)
            let value = try await self.preview(input)
            try validate(input.identity)
            return value
        }, readRevision: { reference in
            try validate(reference.identity)
            let value = try await self.readRevision(reference)
            try validate(reference.identity)
            return value
        }, pendingSubmission: { identity in
            try validate(identity)
            return try self.pendingSubmission(identity)
        }, submit: { input in
            guard self.permitsFollowupActions else {
                return .rejected(String(localized: "Expert form follow-up is not available yet."))
            }
            do { try validate(input.interaction.identity) }
            catch { return .rejected(error.localizedDescription) }
            // Once dispatched, preserve the real receipt/unknown outcome even
            // if the reader navigates away. The original Host settles it.
            return await self.submit(input)
        }, permitsFollowupActions: permitsFollowupActions, readLocalSource: {
            guard isCurrent() else { throw MacApplicationPortError.invalidResponse }
            return try self.readLocalSource()
        })
    }
}

/// One ordinary/expert wrapper binds journal, fallback and historical cards to
/// the same message owner, Host callbacks and native layout.
struct MacMessageSurfaceContent<Content: View>: View {
    let scope: MacMessageSurfaceScope
    let context: MacUISurfaceContext
    let isCurrent: () -> Bool
    @ViewBuilder var content: () -> Content
    @State private var isMounted = false
    var body: some View {
        let scoped = context.scoped(to: scope, isCurrent: { isMounted && isCurrent() })
        VStack(alignment: .leading, spacing: 12) {
            content().environment(\.uiSurfaceContext, isMounted && isCurrent() ? scoped : nil)
        }
        .onAppear { isMounted = true }
        .onDisappear { isMounted = false }
    }
}


/// Both journal and canonical readers mount the same source-bound Surface.
struct MacMessageReferencedSurface: View {
    let reference: AppleUISurfaceReference
    let summary: String
    @Environment(\.uiSurfaceContext) private var context
    var body: some View {
        if let context {
            if context.permitsFollowupActions {
                AppleUIReferencedSurfaceView<MacCanonicalJSONValue>(reference: reference, summary: summary,
                    readChanges: context.readChanges, preview: context.preview, readRevision: context.readRevision,
                    pendingSubmission: context.pendingSubmission, submit: context.submit).id(reference.identity)
            } else {
                AppleUIReadOnlyReferencedSurfaceView<MacCanonicalJSONValue>(reference: reference, summary: summary,
                    notice: String(localized: "Expert form follow-up is not available yet."), readRevision: context.readRevision)
                    .id(reference.identity)
            }
        }
    }
}
