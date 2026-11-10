// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import UIKit
import XCTest
@testable import Nola

@MainActor final class IOSWorkspacePageMatrixTests: XCTestCase {
    func testEveryManagementDestinationAtPhoneAndTabletWidthsInBothThemes() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-pages-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700])
        defer {
            if FileManager.default.fileExists(atPath: root.path) { try? FileManager.default.removeItem(at: root) }
        }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = try LocalConversationStore(applicationRuntime: runtime)
        container.chatManager = ChatManager(container: container, restorePersistedState: false)
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let destinations = IOSWorkspaceDestination.allCases
        for (width, largeText): (CGFloat, Bool) in [(320, false), (390, false), (768, false), (390, true)] {
            for dark in [false, true] {
                for destination in destinations {
                    let view = NavigationStack { IOSWorkspaceManagementContent(destination: destination) }
                        .environmentObject(container)
                        .environment(\.colorScheme, dark ? .dark : .light)
                        .environment(\.dynamicTypeSize, largeText ? .accessibility3 : .large)
                    let controller = UIHostingController(rootView: view)
                    let window = UIWindow(windowScene: scene)
                    window.frame = CGRect(x: 0, y: 0, width: width, height: 900)
                    window.overrideUserInterfaceStyle = dark ? .dark : .light
                    window.rootViewController = controller
                    window.makeKeyAndVisible()
                    defer {
                        window.isHidden = true
                        window.rootViewController = nil
                        window.windowScene = nil
                    }
                    controller.view.frame = window.bounds
                    try await Task.sleep(for: .milliseconds(200))
                    controller.view.layoutIfNeeded()
                    XCTAssertEqual(controller.view.bounds.width, width, accuracy: 0.1)
                    let image = UIGraphicsImageRenderer(size: window.bounds.size).image { _ in
                        controller.view.drawHierarchy(in: controller.view.bounds, afterScreenUpdates: true)
                    }
                    let raster = try XCTUnwrap(image.cgImage)
                    var pixels = [UInt8](repeating: 0, count: 64 * 64 * 4)
                    try pixels.withUnsafeMutableBytes { bytes in
                        let bitmap = try XCTUnwrap(CGContext(data: bytes.baseAddress, width: 64, height: 64,
                            bitsPerComponent: 8, bytesPerRow: 64 * 4, space: CGColorSpaceCreateDeviceRGB(),
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
                        bitmap.draw(raster, in: CGRect(x: 0, y: 0, width: 64, height: 64))
                    }
                    let luminance = stride(from: 0, to: pixels.count, by: 4).map { Int(pixels[$0]) + Int(pixels[$0 + 1]) + Int(pixels[$0 + 2]) }
                    XCTAssertGreaterThan((luminance.max() ?? 0) - (luminance.min() ?? 0), 60,
                        "Page content must actually be painted: \(destination)")
                    let png = try XCTUnwrap(image.pngData())
                    XCTAssertGreaterThan(png.count, 10_000, "Blank management page: \(destination)")
                    // Keep native-resolution evidence without retaining a full
                    // uncompressed bitmap for every page until this test ends.
                    let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
                    attachment.name = "page-\(destination)-\(Int(width))-\(dark ? "dark" : "light")\(largeText ? "-accessibility" : "")"
                    attachment.lifetime = .keepAlways
                    add(attachment)
                    window.isHidden = true
                }
            }
        }
    }
}
