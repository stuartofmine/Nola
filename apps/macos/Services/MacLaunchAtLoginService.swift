// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Connects the General setting to the native macOS login-item service.

import Foundation
import ServiceManagement

enum MacLaunchAtLoginService {
    static var isEnabled: Bool {
        SMAppService.mainApp.status == .enabled
    }

    static func setEnabled(_ enabled: Bool) throws {
        if enabled {
            if SMAppService.mainApp.status != .enabled {
                try SMAppService.mainApp.register()
            }
        } else if SMAppService.mainApp.status == .enabled {
            try SMAppService.mainApp.unregister()
        }
    }
}
