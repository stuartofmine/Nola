import XCTest
import SwiftUI
import UIKit
@testable import Nola

@MainActor
final class WebHostConnectionUIVisualTests: XCTestCase {
    func testConnectionSheetRendersSetupAndInputsAtPhoneSize() throws {
        let view = NavigationStack {
            WebHostConfigurationView(existingModel: nil) { _ in }
        }
        .environment(\.locale, Locale(identifier: "zh-Hans"))

        let controller = UIHostingController(rootView: view)
        let scene = try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        )
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
        window.rootViewController = controller
        window.makeKeyAndVisible()
        controller.view.frame = window.bounds
        controller.view.setNeedsLayout()
        controller.view.layoutIfNeeded()

        let renderer = UIGraphicsImageRenderer(bounds: window.bounds)
        let image = renderer.image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let data = try XCTUnwrap(image.pngData())
        XCTAssertGreaterThan(data.count, 20_000)
        try data.write(to: URL(fileURLWithPath: "/tmp/nola-ios-web-host-connection.png"), options: .atomic)
    }

    func testConnectionFailuresUseStableLocalizedMessages() {
        XCTAssertEqual(
            webHostPairingErrorMessage(WebHostPairingFailure.identityChanged),
            NSLocalizedString("model.web_host.error.identity_changed", comment: "")
        )
        XCTAssertEqual(
            webHostPairingErrorMessage(URLError(.timedOut)),
            NSLocalizedString("model.web_host.error.timeout", comment: "")
        )
        XCTAssertEqual(
            webHostPairingErrorMessage(URLError(.cannotConnectToHost)),
            NSLocalizedString("model.web_host.error.unreachable", comment: "")
        )
    }
}
