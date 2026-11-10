// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the immutable display input and typed action output of the macOS chat composer.

import AppKit
import Foundation

enum MacPermissionConfigurationState: Equatable {
    case draft, checking, saving, saved, awaitingApplication, active, failed

    var title: String {
        switch self {
        case .draft: String(localized: "For the next message", table: "Permissions")
        case .checking: String(localized: "Checking permission status…", table: "Permissions")
        case .saving: String(localized: "Switching permissions…", table: "Permissions")
        case .saved: String(localized: "Permissions saved", table: "Permissions")
        case .awaitingApplication: String(localized: "Saved; waiting for the running task", table: "Permissions")
        case .active: String(localized: "Permissions active", table: "Permissions")
        case .failed: String(localized: "Save not confirmed. Refresh and retry.", table: "Permissions")
        }
    }
}

struct MacComposerDisplayState: Equatable {
    let conversationID: String
    let draft: String
    let attachments: [MacAttachment]
    let isRunning: Bool
    let isVisiblyRunning: Bool
    var reasoningOptions: [String] = []
    var reasoningPreference: String = "automatic"
    var permissionMode: MacPermissionMode = .cautious
    var permissionConfigurationState: MacPermissionConfigurationState = .draft
    var activePermissionMode: MacPermissionMode? = nil
    var isSubmissionBlocked: Bool = false
    var submissionBlockReason: String? = nil

    var canSend: Bool {
        guard !isSubmissionBlocked, !isRunning else { return false }
        let hasContent = !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !attachments.isEmpty
        return hasContent && !attachments.contains(where: { $0.status == .extracting })
    }

    var isPrimaryActionDisabled: Bool {
        (isSubmissionBlocked && !isVisiblyRunning)
            || (isRunning && !isVisiblyRunning) || (!canSend && !isVisiblyRunning)
    }
}

enum MacComposerAction {
    case updateDraft(conversationID: String, value: String)
    case submit(conversationID: String)
    case stop(conversationID: String)
    case importFiles(conversationID: String, urls: [URL])
    case importDroppedAttachments(conversationID: String, providers: [NSItemProvider])
    case importPasteboardAttachments(conversationID: String, pasteboard: NSPasteboard)
    case removeAttachment(conversationID: String, attachmentID: UUID)
    case selectReasoningPreference(conversationID: String, String)
    case selectPermissionMode(conversationID: String, MacPermissionMode)
    case reloadPermissionConfiguration(conversationID: String)
}

/// A reference-typed sink keeps the action endpoint stable while immutable
/// display snapshots change during typing, extraction, and run transitions.
@MainActor
final class MacComposerActionSink {
    typealias Handler = @MainActor (MacComposerAction) -> Bool

    private let handler: Handler

    init(handler: @escaping Handler) {
        self.handler = handler
    }

    @discardableResult
    func perform(_ action: MacComposerAction) -> Bool {
        handler(action)
    }
}
