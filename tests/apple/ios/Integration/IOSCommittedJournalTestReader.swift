// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

/// Integration assertions consume the original admitted Run's sealed journal,
/// never the diagnostic trace and never a second model invocation.
@MainActor
final class IOSCommittedJournalTestReader {
    private var receipt: IOSSourceSubmissionReceipt?

    func observe(_ event: WebAgentStreamEvent) throws {
        guard case .submissionReceipt(let raw) = event else { return }
        let value = try JSONDecoder().decode(IOSSourceSubmissionReceipt.self, from: JSONEncoder().encode(raw))
        guard value.protocol_version == 1 else { throw IOSApplicationRuntimeError.rejected("Invalid receipt version") }
        if let receipt {
            guard receipt.identity == value.identity, receipt.request_digest == value.request_digest else {
                throw IOSApplicationRuntimeError.rejected("Integration stream changed submission identity")
            }
        }
        if value.phase == "admitted" { receipt = value }
    }

    func text(provider: WebAgentProvider) async throws -> String {
        let receipt = try XCTUnwrap(receipt)
        let admission = try XCTUnwrap(receipt.admission)
        guard admission.identity.conversation_id == receipt.identity.conversation_id,
              admission.identity.run_id == receipt.identity.submission_id,
              admission.identity.assistant_message_id == receipt.identity.assistant_message_id else {
            throw IOSApplicationRuntimeError.rejected("Receipt does not own its journal")
        }
        return try await provider.readCompletedSummary(admission: admission, expectedSealDigest: nil)
    }
}
