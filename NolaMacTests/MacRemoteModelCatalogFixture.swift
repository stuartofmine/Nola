// SPDX-License-Identifier: Apache-2.0
import Foundation
@testable import Nola

extension MacWorkspaceModel {
    /// Scripted Host fixture: the first supplied profile is this test Host's
    /// published automatic default, not a production client fallback policy.
    @MainActor
    func setFixtureRemoteModels(_ profiles: [MacRemoteModelProfile]) {
        guard let first = profiles.first else { updateRemoteModelCatalog(nil); return }
        updateRemoteModelCatalog(.init(hostID: first.hostID, profiles: profiles,
            defaultResolution: .resolved(profileID: first.profileID, origin: .automatic), defaultRevision: nil))
    }
}
