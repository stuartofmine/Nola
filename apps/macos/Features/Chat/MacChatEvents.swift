// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines process-local integration events shared by the macOS chat feature.

import Foundation

extension Notification.Name {
    static let macBrowserActivityDidChange = Notification.Name(
        "NolaMac.browserActivityDidChange"
    )

    static let macCompactionSourceRequested = Notification.Name(
        "NolaMac.CompactionSourceRequested"
    )
}
