// SPDX-License-Identifier: Apache-2.0
import XCTest

/// Real app navigation with a fresh namespace; no provider, permission grants or inference.
final class IOSWorkspaceNavigationSmokeTests: XCTestCase {
    func testManagementPagesAndNavigationInBothThemesWithoutModelAccount() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "navigation-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["main.tab.chat"].waitForExistence(timeout: 10))

        func tap(_ identifier: String) {
            let button = app.buttons[identifier]
            XCTAssertTrue(button.waitForExistence(timeout: 5), identifier)
            for _ in 0..<5 where !button.isHittable { app.swipeUp() }
            for _ in 0..<5 where !button.isHittable { app.swipeDown() }
            XCTAssertTrue(button.isHittable, identifier)
            button.tap()
        }
        func capture(_ name: String) {
            let attachment = XCTAttachment(data: app.screenshot().pngRepresentation, uniformTypeIdentifier: "public.png")
            attachment.name = name
            attachment.lifetime = .keepAlways
            add(attachment)
        }
        func backToSettings() {
            let back = app.navigationBars.buttons.element(boundBy: 0)
            XCTAssertTrue(back.waitForExistence(timeout: 3))
            back.tap()
            XCTAssertTrue(app.buttons["settings.models"].waitForExistence(timeout: 3))
        }

        for theme in ["Light", "Dark"] {
            tap("main.tab.settings")
            tap("settings.category.appearance")
            tap("settings.appearance")
            let choice = app.buttons[theme]
            XCTAssertTrue(choice.waitForExistence(timeout: 3))
            choice.tap()
            capture("appearance-\(theme)")
            backToSettings()

            tap("settings.category.agents")
            XCTAssertTrue(app.buttons["experts.management.done"].waitForExistence(timeout: 5))
            capture("experts-\(theme)")
            tap("experts.management.done")

            for identifier in ["settings.models", "settings.category.agent", "settings.category.notifications",
                               "settings.category.updates", "settings.category.privacy", "settings.category.memory", "settings.category.plans"] {
                tap(identifier)
                capture("\(identifier)-\(theme)")
                backToSettings()
            }
            for destination in ["knowledge", "memory", "sources"] {
                tap("main.tab.information")
                tap("information.\(destination)")
                capture("library-\(destination)-\(theme)")
                let back = app.navigationBars.buttons.element(boundBy: 0)
                XCTAssertTrue(back.waitForExistence(timeout: 3))
                back.tap()
                XCTAssertTrue(app.buttons["information.knowledge"].waitForExistence(timeout: 3))
            }
            for tab in ["chat", "activity", "information", "settings"] {
                tap("main.tab.\(tab)")
                capture("primary-\(tab)-\(theme)")
            }
        }
    }
}
