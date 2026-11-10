// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import UIKit
import XCTest
@testable import Nola

@MainActor final class IOSOnboardingGuidanceTests: XCTestCase {
    func testAllGuidePagesRenderWithoutChangingPermissionsOrSavedExecutionPreferences() async throws {
        let suite = "onboarding-guidance-\(UUID())"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { preferences.removePersistentDomain(forName: suite) }
        preferences.set("existing-host", forKey: "onboarding.runtime.preference")
        preferences.set("existing-sources", forKey: "onboarding.planned.sources")
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        for (width, largeText): (CGFloat, Bool) in [(320, false), (390, true), (768, false)] {
            for dark in [false, true] {
                for step in 0..<4 {
                    let controller = UIHostingController(rootView: OnboardingView(initialStep: step)
                        .defaultAppStorage(preferences)
                        .environment(\.colorScheme, dark ? .dark : .light)
                        .environment(\.dynamicTypeSize, largeText ? .accessibility3 : .large)
                        .transaction { $0.animation = nil; $0.disablesAnimations = true })
                    let window = UIWindow(windowScene: scene)
                    window.frame = CGRect(x: 0, y: 0, width: width, height: 900)
                    window.overrideUserInterfaceStyle = dark ? .dark : .light
                    window.rootViewController = controller; window.makeKeyAndVisible()
                    defer { window.isHidden = true; window.rootViewController = nil; window.windowScene = nil }
                    try await Task.sleep(for: .milliseconds(100))
                    controller.view.layoutIfNeeded()
                    let image = UIGraphicsImageRenderer(size: window.bounds.size).image { _ in
                        controller.view.drawHierarchy(in: controller.view.bounds, afterScreenUpdates: true)
                    }
                    let png = try XCTUnwrap(image.pngData())
                    XCTAssertGreaterThan(png.count, 10_000)
                    try assertFooterUsesThemeActionColor(image, dark: dark)
                    let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
                    attachment.name = "onboarding-\(step)-\(Int(width))-\(dark ? "dark" : "light")\(largeText ? "-accessibility" : "")"
                    attachment.lifetime = .keepAlways; add(attachment)
                    XCTAssertFalse(preferences.bool(forKey: "hasCompletedOnboarding"))
                    XCTAssertEqual(preferences.string(forKey: "onboarding.runtime.preference"), "existing-host")
                    XCTAssertEqual(preferences.string(forKey: "onboarding.planned.sources"), "existing-sources")
                    window.isHidden = true; window.rootViewController = nil; window.windowScene = nil
                }
            }
        }
    }

    private func assertFooterUsesThemeActionColor(_ image: UIImage, dark: Bool) throws {
        let expected = UIColor(DesignTokens.actionColor).resolvedColor(with: UITraitCollection(userInterfaceStyle: dark ? .dark : .light))
        var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
        XCTAssertTrue(expected.getRed(&red, green: &green, blue: &blue, alpha: &alpha))
        let expectedBytes = [red, green, blue].map { Double($0) * 255 }
        var pixels = [UInt8](repeating: 0, count: 64 * 64 * 4)
        let raster = try XCTUnwrap(image.cgImage)
        try pixels.withUnsafeMutableBytes { bytes in
            let context = try XCTUnwrap(CGContext(data: bytes.baseAddress, width: 64, height: 64,
                bitsPerComponent: 8, bytesPerRow: 64 * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue))
            context.draw(raster, in: CGRect(x: 0, y: 0, width: 64, height: 64))
        }
        // The footer occupies the bottom quarter. Allow system button materials
        // and antialiasing while rejecting an untinted white prominent button.
        let matches = stride(from: 48 * 64 * 4, to: pixels.count, by: 4).filter { index in
            (0..<3).allSatisfy { abs(Double(pixels[index + $0]) - expectedBytes[$0]) < 40 }
        }.count
        XCTAssertGreaterThan(matches, 10, "The rendered primary action must use the theme color in both appearances")
    }
}
