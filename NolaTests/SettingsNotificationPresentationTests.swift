// SPDX-License-Identifier: Apache-2.0
// Purpose: Verifies notification permission decisions presented by iOS Settings.

import UserNotifications
import XCTest
@testable import Nola

final class SettingsNotificationPresentationTests: XCTestCase {
    func testDeniedAuthorizationRequiresSystemSettingsInsteadOfAnotherPrompt() {
        XCTAssertTrue(SettingsNotificationPresentation.requiresSystemSettings(
            requestedEnabled: true,
            authorizationStatus: .denied
        ))
    }

    func testAuthorizedAndUndeterminedStatesDoNotRequireSystemSettings() {
        XCTAssertFalse(SettingsNotificationPresentation.requiresSystemSettings(
            requestedEnabled: true,
            authorizationStatus: .authorized
        ))
        XCTAssertFalse(SettingsNotificationPresentation.requiresSystemSettings(
            requestedEnabled: true,
            authorizationStatus: .notDetermined
        ))
    }

    func testTurningNotificationsOffNeverOpensSystemSettings() {
        XCTAssertFalse(SettingsNotificationPresentation.requiresSystemSettings(
            requestedEnabled: false,
            authorizationStatus: .denied
        ))
    }
}
