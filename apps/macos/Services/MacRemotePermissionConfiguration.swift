// SPDX-License-Identifier: Apache-2.0
// Current source-Host configuration only. A saved profile is not a policy ACK.
import Foundation

nonisolated struct MacRemotePermissionConfiguration: Decodable, Sendable {
    let conversation_id: String
    let configuration_revision: Int64
    let permission_profile: MacPermissionMode?
    let active_run_id: String?
    let active_policy: MacActivePermissionPolicy?
    enum ApplicationState: String, Decodable, Sendable { case inactive, pending, active }
    struct Owner: Decodable, Sendable {
        enum Kind: String, Decodable, Sendable { case inactive, local, remote }
        let kind: Kind
        let run_id: String?
        let assistant_message_id: String?
        let identity: MacRemoteSubmissionIdentity?
    }
    let owner: Owner
    let state: ApplicationState

    func validate(identity: MacRemoteSubmissionIdentity) throws {
        switch owner.kind {
        case .inactive:
            guard active_run_id == nil, state == .inactive else { throw MacRemoteHostError.invalidResponse }
        case .local:
            guard owner.run_id == identity.submission_id, owner.run_id == active_run_id,
                  owner.assistant_message_id == identity.assistant_message_id else { throw MacRemoteHostError.invalidResponse }
        case .remote:
            guard let source = owner.identity, source == identity,
                  source.submission_id == identity.submission_id, source.submission_id == active_run_id else { throw MacRemoteHostError.invalidResponse }
        }
        guard conversation_id == identity.conversation_id, configuration_revision > 0,
              active_run_id == nil || active_run_id == identity.submission_id else {
            throw MacRemoteHostError.invalidResponse
        }
        if let active_policy {
            guard active_policy.runID == active_run_id,
                  active_policy.configurationRevision > 0,
                  active_policy.configurationRevision <= configuration_revision else {
                throw MacRemoteHostError.invalidResponse
            }
        }
    }
}
