// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the typed macOS Application boundary for Activity history queries.

import Foundation
import SwiftUI

nonisolated protocol MacActivityApplicationClient: Sendable {
    func listActivity(_ query: AppleActivityQuery) throws -> AppleActivityPage
    func activityDetail(_ entry: AppleActivityEntry) throws -> AppleActivityDetail?
}

extension MacApplicationPort: MacActivityApplicationClient {}

nonisolated enum MacActivityApplicationComposition {
    static let sharedClient: any MacActivityApplicationClient = MacApplicationPort.shared

    static func client(databaseURL: URL) -> (any MacActivityApplicationClient)? {
        try? MacApplicationPort(databaseURL: databaseURL)
    }
}

private struct MacActivityApplicationClientKey: EnvironmentKey {
    static let defaultValue: any MacActivityApplicationClient =
        MacActivityApplicationComposition.sharedClient
}

extension EnvironmentValues {
    var macActivityApplicationClient: any MacActivityApplicationClient {
        get { self[MacActivityApplicationClientKey.self] }
        set { self[MacActivityApplicationClientKey.self] = newValue }
    }
}
