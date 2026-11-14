import XCTest
import SwiftUI
import UIKit
import Vision
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
        let recognition = VNRecognizeTextRequest()
        recognition.recognitionLanguages = ["zh-Hans", "en-US"]
        recognition.recognitionLevel = .accurate
        try VNImageRequestHandler(cgImage: XCTUnwrap(image.cgImage)).perform([recognition])
        let visibleText = recognition.results?.compactMap { $0.topCandidates(1).first?.string }
            .joined().filter { !$0.isWhitespace } ?? ""
        let pasteLabel = NSLocalizedString("model.web_host.paste_setup", value: "Paste Connection Setup", comment: "")
            .filter { !$0.isWhitespace }
        XCTAssertTrue(visibleText.contains(pasteLabel), "The pairing action must be visible, not an empty control row")
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
    func testSavedPairingTokenNeverLoadsForAnEditedDestination() throws {
        let original = "https://host.example/routed"
        var reads = 0
        for changed in ["https://other.example/routed", "https://host.example:8443/routed",
                        "http://host.example/routed", "https://host.example/other"] {
            let url = try XCTUnwrap(IOSHostConnectionDraftPolicy.endpoint(changed))
            let token = IOSHostConnectionDraftPolicy.token(entered: "", for: url, original: original) {
                reads += 1
                return "saved-secret"
            }
            XCTAssertEqual(token, "", changed)
        }
        XCTAssertEqual(reads, 0, "An edited destination must not even read the saved credential")
        let approved = try XCTUnwrap(IOSHostConnectionDraftPolicy.endpoint(original))
        XCTAssertEqual(IOSHostConnectionDraftPolicy.token(entered: "", for: approved, original: original) {
            reads += 1; return "saved-secret"
        }, "saved-secret")
        XCTAssertEqual(reads, 1)
        XCTAssertEqual(IOSHostConnectionDraftPolicy.token(entered: " replacement ", for: approved, original: original) {
            XCTFail("Explicit pairing token must not read a saved secret"); return nil
        }, "replacement")
    }

    func testManualPairingRejectsAmbiguousCredentialDestinations() {
        for input in ["https://host.example?token=x", "https://host.example#fragment",
                      "https://user:password@host.example", "https://host.example:0",
                      "https://host.example:65536", "file:///tmp/host"] {
            XCTAssertNil(IOSHostConnectionDraftPolicy.endpoint(input), input)
        }
        XCTAssertNotNil(IOSHostConnectionDraftPolicy.endpoint("http://192.168.1.2:8080"))
    }

    func testPermissionCallbackRequiresTheOriginalStepAndPresentation() async {
        let owner = UUID()
        // Simulate returning from an asynchronous OS request after the UI moved
        // or closed. The production completion uses this same ownership gate.
        await Task.yield()
        XCTAssertTrue(PermissionSetupRequestCompletion.accepts(sourceIndex: 0, currentIndex: 0,
            owner: owner, activeOwner: owner, cancelled: false))
        XCTAssertFalse(PermissionSetupRequestCompletion.accepts(sourceIndex: 0, currentIndex: 1,
            owner: owner, activeOwner: owner, cancelled: false))
        XCTAssertFalse(PermissionSetupRequestCompletion.accepts(sourceIndex: 0, currentIndex: 0,
            owner: owner, activeOwner: nil, cancelled: false))
        XCTAssertFalse(PermissionSetupRequestCompletion.accepts(sourceIndex: 0, currentIndex: 0,
            owner: owner, activeOwner: UUID(), cancelled: false))
        XCTAssertFalse(PermissionSetupRequestCompletion.accepts(sourceIndex: 0, currentIndex: 0,
            owner: owner, activeOwner: owner, cancelled: true))
    }

}
