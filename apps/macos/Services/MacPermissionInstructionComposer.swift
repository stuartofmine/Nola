// SPDX-License-Identifier: Apache-2.0
import Foundation
import Observation

/// Permission-card state only. Guidance is never submitted through chat input.
@MainActor
@Observable
final class MacPermissionInstructionComposer {
    var text = ""
    private(set) var isSending = false
    private(set) var error: String?
    private(set) var submittedRequestID: String?

    func validationError(request: MacRemotePermissionRequest) -> String? {
        guard !text.isEmpty else { return nil }
        do {
            _ = try request.instructionDecision(text)
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    func canSubmit(request: MacRemotePermissionRequest) -> Bool {
        !isSending && submittedRequestID == nil && (try? request.instructionDecision(text)) != nil
    }

    func submit(request: MacRemotePermissionRequest, send: @MainActor (String) async throws -> Void) async {
        guard canSubmit(request: request) else { return }
        let instruction = text.trimmingCharacters(in: .whitespacesAndNewlines)
        isSending = true
        error = nil
        defer { isSending = false }
        do {
            try await send(instruction)
            submittedRequestID = request.id
            text = ""
        } catch {
            self.error = error.localizedDescription
        }
    }
}
