// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Real UIWindow / UIKit sheet sRGB CGImage pixel, safe-area, theme leak, and contrast assertions for iOS SwiftUI views.

import CoreGraphics
import Foundation
import SwiftUI
import UIKit
import XCTest
@testable import Nola

struct IOSVisualPixelSummary {
    let width: Int
    let height: Int
    let analyzedPixels: Int
    let opaqueFraction: Double
    let warmSurfaceFraction: Double
    let pureBlackFraction: Double
    let fullWidthBlackScanlines: Int
    let coolGrayFraction: Double
    let foregroundContrastSpread: Int
    let headerContrastSpread: Int
    let bodyContrastSpread: Int
    let topSafeWarmFraction: Double?
    let bottomSafeWarmFraction: Double?
}

@MainActor
enum IOSVisualPixelAssertions {
    private struct RGBPixel {
        let r: Int
        let g: Int
        let b: Int
    }

    private static let lightSurfaceTargets: [RGBPixel] = [
        RGBPixel(r: 250, g: 250, b: 250), // #FAFAFA Porcelain canvas
        RGBPixel(r: 255, g: 255, b: 255), // #FFFFFF surface / elevated surface
        RGBPixel(r: 242, g: 242, b: 244)  // #F2F2F4 subtle surface
    ]

    private static let darkSurfaceTargets: [RGBPixel] = [
        RGBPixel(r: 11, g: 11, b: 12), // #0B0B0C Monochrome Obsidian canvas
        RGBPixel(r: 19, g: 19, b: 21), // #131315 surface
        RGBPixel(r: 26, g: 26, b: 29), // #1A1A1D subtle surface
        RGBPixel(r: 36, g: 36, b: 40)  // #242428 elevated surface
    ]

    private static let lightNeutralAccentTargets: [RGBPixel] = [
        RGBPixel(r: 235, g: 235, b: 239), // #EBEBEF accentSubtle
        RGBPixel(r: 228, g: 228, b: 231)  // #E4E4E7 border
    ]

    private static let darkNeutralAccentTargets: [RGBPixel] = [
        RGBPixel(r: 32, g: 32, b: 36), // #202024 accentSubtle
        RGBPixel(r: 38, g: 38, b: 42)  // #26262A border
    ]

    static func findRepositoryRoot(from filePath: StaticString = #filePath) throws -> URL {
        var cursor = URL(fileURLWithPath: "\(filePath)").deletingLastPathComponent()
        for _ in 0..<10 {
            let candidate = cursor.appendingPathComponent("shared/design-tokens/tokens.json")
            if FileManager.default.fileExists(atPath: candidate.path) {
                return cursor
            }
            let parent = cursor.deletingLastPathComponent()
            if parent.path == cursor.path { break }
            cursor = parent
        }
        throw NSError(
            domain: "IOSVisualPixelAssertions",
            code: 404,
            userInfo: [NSLocalizedDescriptionKey: "Could not locate repository root containing shared/design-tokens/tokens.json"]
        )
    }

    static func currentWindowScene() throws -> UIWindowScene {
        try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first,
            "Active UIWindowScene required for iOS visual rendering tests"
        )
    }

    static func makeWindow(
        size: CGSize,
        safeAreaInsets: UIEdgeInsets = .zero,
        userInterfaceStyle: UIUserInterfaceStyle,
        horizontalSizeClass: UIUserInterfaceSizeClass? = nil,
        rootViewController: UIViewController
    ) throws -> UIWindow {
        let scene = try currentWindowScene()
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(origin: .zero, size: size)
        window.overrideUserInterfaceStyle = userInterfaceStyle
        if #available(iOS 17.0, *), let horizontalSizeClass {
            rootViewController.traitOverrides.horizontalSizeClass = horizontalSizeClass
        }
        if safeAreaInsets != .zero {
            let existing = rootViewController.view.safeAreaInsets
            rootViewController.additionalSafeAreaInsets = UIEdgeInsets(
                top: max(0, safeAreaInsets.top - existing.top),
                left: max(0, safeAreaInsets.left - existing.left),
                bottom: max(0, safeAreaInsets.bottom - existing.bottom),
                right: max(0, safeAreaInsets.right - existing.right)
            )
        }
        window.rootViewController = rootViewController
        window.makeKeyAndVisible()
        rootViewController.view.frame = window.bounds
        return window
    }

    static func teardownWindow(_ window: UIWindow) {
        if let presented = window.rootViewController?.presentedViewController {
            presented.dismiss(animated: false)
        }
        window.isHidden = true
        window.rootViewController = nil
        window.windowScene = nil
    }

    static func settleLayout(on window: UIWindow, milliseconds: UInt64 = 180) async throws {
        window.setNeedsLayout()
        window.layoutIfNeeded()
        await Task.yield()
        try await Task.sleep(nanoseconds: milliseconds * 1_000_000)
        window.setNeedsLayout()
        window.layoutIfNeeded()
        await Task.yield()
    }

    static func captureImage(of view: UIView, scale: CGFloat = 2.0) throws -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = scale
        format.opaque = false
        let renderer = UIGraphicsImageRenderer(bounds: view.bounds, format: format)
        return renderer.image { _ in
            _ = view.drawHierarchy(in: view.bounds, afterScreenUpdates: true)
        }
    }

    static func captureWindowImage(_ window: UIWindow, scale: CGFloat = 2.0) throws -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = scale
        format.opaque = true
        let renderer = UIGraphicsImageRenderer(bounds: window.bounds, format: format)
        return renderer.image { _ in
            _ = window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
    }

    @discardableResult
    static func assertHostedView<Content: View>(
        _ view: Content,
        label: String,
        size: CGSize,
        safeAreaInsets: UIEdgeInsets = .zero,
        userInterfaceStyle: UIUserInterfaceStyle,
        horizontalSizeClass: UIUserInterfaceSizeClass? = nil,
        dynamicTypeSize: DynamicTypeSize = .large,
        locale: Locale = Locale(identifier: "zh-Hans"),
        settleMilliseconds: UInt64 = 200,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws -> UIImage {
        let wrapped = view
            .environment(\.colorScheme, userInterfaceStyle == .dark ? .dark : .light)
            .environment(\.dynamicTypeSize, dynamicTypeSize)
            .environment(\.locale, locale)
        let controller = UIHostingController(rootView: wrapped)
        let window = try makeWindow(
            size: size,
            safeAreaInsets: safeAreaInsets,
            userInterfaceStyle: userInterfaceStyle,
            horizontalSizeClass: horizontalSizeClass,
            rootViewController: controller
        )
        defer { teardownWindow(window) }

        try await settleLayout(on: window, milliseconds: settleMilliseconds)
        let image = try captureWindowImage(window)
        let cgImage = try XCTUnwrap(image.cgImage, "Missing CGImage for \(label)", file: file, line: line)
        _ = try assertCGImage(
            cgImage,
            label: label,
            isDark: userInterfaceStyle == .dark,
            safeAreaTopPoints: safeAreaInsets.top,
            safeAreaBottomPoints: safeAreaInsets.bottom,
            pointSize: window.bounds.size,
            file: file,
            line: line
        )
        maybeSaveArtifact(image: image, name: label)
        return image
    }

    @discardableResult
    static func assertPresentedSheet<Content: View>(
        _ sheet: Content,
        label: String,
        size: CGSize = CGSize(width: 390, height: 844),
        userInterfaceStyle: UIUserInterfaceStyle,
        locale: Locale = Locale(identifier: "zh-Hans"),
        dynamicTypeSize: DynamicTypeSize = .large,
        settleMilliseconds: UInt64 = 220,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws -> UIImage {
        let hostRoot = UIViewController()
        hostRoot.view.backgroundColor = UIColor(DesignTokens.backgroundColor)
        let window = try makeWindow(
            size: size,
            userInterfaceStyle: userInterfaceStyle,
            rootViewController: hostRoot
        )
        defer { teardownWindow(window) }
        try await settleLayout(on: window, milliseconds: 50)

        let wrappedSheet = sheet
            .environment(\.colorScheme, userInterfaceStyle == .dark ? .dark : .light)
            .environment(\.dynamicTypeSize, dynamicTypeSize)
            .environment(\.locale, locale)
        let sheetController = UIHostingController(rootView: wrappedSheet)
        sheetController.modalPresentationStyle = .pageSheet

        hostRoot.present(sheetController, animated: false)
        let deadline = Date().addingTimeInterval(2.0)
        while Date() < deadline {
            await Task.yield()
            if hostRoot.presentedViewController === sheetController,
               !sheetController.isBeingPresented,
               sheetController.view.window != nil {
                break
            }
            try await Task.sleep(nanoseconds: 20_000_000)
        }

        let presented = try XCTUnwrap(
            hostRoot.presentedViewController,
            "Sheet did not present for \(label)",
            file: file,
            line: line
        )
        try await settleLayout(on: window, milliseconds: settleMilliseconds)
        presented.view.setNeedsLayout()
        presented.view.layoutIfNeeded()
        XCTAssertGreaterThan(presented.view.bounds.width, 200, "\(label): sheet bounds collapsed", file: file, line: line)
        XCTAssertGreaterThan(presented.view.bounds.height, 200, "\(label): sheet height collapsed", file: file, line: line)

        let image = try captureImage(of: presented.view)
        let cgImage = try XCTUnwrap(image.cgImage, "Missing CGImage for presented sheet \(label)", file: file, line: line)
        _ = try assertCGImage(
            cgImage,
            label: label,
            isDark: userInterfaceStyle == .dark,
            pointSize: presented.view.bounds.size,
            file: file,
            line: line
        )
        maybeSaveArtifact(image: image, name: label)
        return image
    }

    @discardableResult
    static func assertCGImage(
        _ cgImage: CGImage,
        label: String,
        isDark: Bool,
        safeAreaTopPoints: CGFloat = 0,
        safeAreaBottomPoints: CGFloat = 0,
        pointSize: CGSize? = nil,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws -> IOSVisualPixelSummary {
        let summary = try analyze(
            cgImage: cgImage,
            isDark: isDark,
            safeAreaTopPoints: safeAreaTopPoints,
            safeAreaBottomPoints: safeAreaBottomPoints,
            pointSize: pointSize
        )

        XCTAssertGreaterThanOrEqual(
            summary.opaqueFraction, 0.70,
            "\(label): rendered view must be mostly opaque Warm Paper/Ink surface (opaque=\(formatPct(summary.opaqueFraction)))",
            file: file, line: line
        )
        XCTAssertGreaterThanOrEqual(
            summary.warmSurfaceFraction, 0.35,
            "\(label): dominant background must match Warm \(isDark ? "Ink" : "Paper") palette (warm=\(formatPct(summary.warmSurfaceFraction)))",
            file: file, line: line
        )
        if isDark {
            XCTAssertLessThan(
                summary.pureBlackFraction, 0.005,
                "\(label): detected OLED #000000 black background leak (\(formatPct(summary.pureBlackFraction)))",
                file: file, line: line
            )
            XCTAssertEqual(
                summary.fullWidthBlackScanlines, 0,
                "\(label): detected \(summary.fullWidthBlackScanlines) full-width pure-black scanline(s)",
                file: file, line: line
            )
        }
        XCTAssertLessThan(
            summary.coolGrayFraction, 0.01,
            "\(label): detected cool UIKit grouped gray leak such as #F2F2F7/#1C1C1E (\(formatPct(summary.coolGrayFraction)))",
            file: file, line: line
        )
        XCTAssertGreaterThanOrEqual(
            summary.foregroundContrastSpread, 40,
            "\(label): rendered pixels lack visible foreground contrast (spread=\(summary.foregroundContrastSpread)/255)",
            file: file, line: line
        )
        XCTAssertGreaterThanOrEqual(
            max(summary.headerContrastSpread, summary.bodyContrastSpread), 40,
            "\(label): header (\(summary.headerContrastSpread)) and body (\(summary.bodyContrastSpread)) both appear blank",
            file: file, line: line
        )

        if let topWarm = summary.topSafeWarmFraction {
            XCTAssertGreaterThanOrEqual(
                topWarm, 0.45,
                "\(label): top safe-area inset does not continue Warm \(isDark ? "Ink" : "Paper") surface (\(formatPct(topWarm)))",
                file: file, line: line
            )
        }
        if let bottomWarm = summary.bottomSafeWarmFraction {
            XCTAssertGreaterThanOrEqual(
                bottomWarm, 0.45,
                "\(label): bottom safe-area inset does not continue Warm \(isDark ? "Ink" : "Paper") surface (\(formatPct(bottomWarm)))",
                file: file, line: line
            )
        }
        return summary
    }

    static func analyze(
        cgImage: CGImage,
        isDark: Bool,
        safeAreaTopPoints: CGFloat = 0,
        safeAreaBottomPoints: CGFloat = 0,
        pointSize: CGSize? = nil
    ) throws -> IOSVisualPixelSummary {
        let width = cgImage.width
        let height = cgImage.height
        guard width > 0, height > 0 else {
            throw NSError(domain: "IOSVisualPixelAssertions", code: 1, userInfo: [NSLocalizedDescriptionKey: "Empty CGImage"])
        }
        guard let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) else {
            throw NSError(domain: "IOSVisualPixelAssertions", code: 2, userInfo: [NSLocalizedDescriptionKey: "Unavailable sRGB CGColorSpace"])
        }

        var buffer = [UInt8](repeating: 0, count: width * height * 4)
        let bytesPerRow = width * 4
        let bitmapInfo = CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue
        let drew = buffer.withUnsafeMutableBytes { raw -> Bool in
            guard let ctx = CGContext(
                data: raw.baseAddress,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: bytesPerRow,
                space: colorSpace,
                bitmapInfo: bitmapInfo
            ) else {
                return false
            }
            ctx.draw(cgImage, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        guard drew else {
            throw NSError(domain: "IOSVisualPixelAssertions", code: 3, userInfo: [NSLocalizedDescriptionKey: "Failed to rasterize CGImage into sRGB buffer"])
        }

        let scaleY: CGFloat = if let pointSize, pointSize.height > 0 {
            CGFloat(height) / pointSize.height
        } else {
            2.0
        }
        let topSafeRows = min(height, max(0, Int((safeAreaTopPoints * scaleY).rounded(.down))))
        let bottomSafeRows = min(height - topSafeRows, max(0, Int((safeAreaBottomPoints * scaleY).rounded(.down))))
        let headerStart = topSafeRows
        let headerEnd = min(height - bottomSafeRows, headerStart + max(1, Int((150 * scaleY).rounded())))

        let surfaceTargets = isDark ? darkSurfaceTargets : lightSurfaceTargets
        var opaqueCount = 0
        var warmSurfaceCount = 0
        var pureBlackCount = 0
        var fullWidthBlackScanlines = 0
        var coolGrayCount = 0
        var topSafeOpaque = 0
        var topSafeWarm = 0
        var bottomSafeOpaque = 0
        var bottomSafeWarm = 0

        var fullHist = [Int](repeating: 0, count: 256)
        var headerHist = [Int](repeating: 0, count: 256)
        var bodyHist = [Int](repeating: 0, count: 256)

        for y in 0..<height {
            var rowBlackCount = 0
            var rowOpaqueCount = 0
            let inTopSafe = topSafeRows > 0 && y < topSafeRows
            let inBottomSafe = bottomSafeRows > 0 && y >= (height - bottomSafeRows)
            let inHeader = y >= headerStart && y < headerEnd
            let inBody = y >= headerEnd && y < (height - bottomSafeRows)

            for x in 0..<width {
                let offset = (y * width + x) * 4
                let a = Int(buffer[offset + 3])
                if a < 16 { continue }

                // Unpremultiply so translucent pixels over transparent corners still reflect true RGB.
                let r = min(255, a == 255 ? Int(buffer[offset]) : (Int(buffer[offset]) * 255) / a)
                let g = min(255, a == 255 ? Int(buffer[offset + 1]) : (Int(buffer[offset + 1]) * 255) / a)
                let b = min(255, a == 255 ? Int(buffer[offset + 2]) : (Int(buffer[offset + 2]) * 255) / a)

                opaqueCount += 1
                rowOpaqueCount += 1

                let isWarm = isWarmSurfacePixel(r: r, g: g, b: b, targets: surfaceTargets, tolerance: 2)
                if isWarm { warmSurfaceCount += 1 }
                if inTopSafe {
                    topSafeOpaque += 1
                    if isWarm { topSafeWarm += 1 }
                }
                if inBottomSafe {
                    bottomSafeOpaque += 1
                    if isWarm { bottomSafeWarm += 1 }
                }

                if r <= 3 && g <= 3 && b <= 3 {
                    pureBlackCount += 1
                    rowBlackCount += 1
                }

                let neutralAccentTargets = isDark ? darkNeutralAccentTargets : lightNeutralAccentTargets
                let isApprovedNeutral = isWarm || isWarmSurfacePixel(r: r, g: g, b: b, targets: neutralAccentTargets, tolerance: 2)
                let isNeutralCoolGray: Bool
                if isDark {
                    // Monochrome Obsidian dark ramp (#0B0B0C..#26262A) naturally sits at b - r == 1..4;
                    // flag genuine cool-blue leaks (b - r >= 6) matching XCUIElementPixelAssertions.
                    isNeutralCoolGray = abs(r - g) <= 2 && (
                        ((22...34).contains(r) && (b - r) >= 6) ||
                        ((35...55).contains(r) && (b - r) >= 6)
                    )
                } else {
                    // Porcelain surfaces (#FAFAFA/#FFFFFF/#F2F2F4), accentSubtle (#EBEBEF), and border (#E4E4E7)
                    // are approved tokens; flag UIKit grouped gray leaks such as #F2F2F7 (b - r == 5).
                    isNeutralCoolGray = r > 220 && (b - r) >= 4 && abs(r - g) <= 4
                }
                if isNeutralCoolGray && !isApprovedNeutral {
                    coolGrayCount += 1
                }

                let luma = min(255, max(0, (299 * r + 587 * g + 114 * b) / 1000))
                fullHist[luma] += 1
                if inHeader { headerHist[luma] += 1 }
                if inBody { bodyHist[luma] += 1 }
            }

            if rowOpaqueCount > width / 2 && rowBlackCount == rowOpaqueCount {
                fullWidthBlackScanlines += 1
            }
        }

        let totalPixels = width * height
        let safeDenom = max(1, opaqueCount)
        return IOSVisualPixelSummary(
            width: width,
            height: height,
            analyzedPixels: totalPixels,
            opaqueFraction: Double(opaqueCount) / Double(max(1, totalPixels)),
            warmSurfaceFraction: Double(warmSurfaceCount) / Double(safeDenom),
            pureBlackFraction: Double(pureBlackCount) / Double(safeDenom),
            fullWidthBlackScanlines: fullWidthBlackScanlines,
            coolGrayFraction: Double(coolGrayCount) / Double(safeDenom),
            foregroundContrastSpread: contrastSpread(from: fullHist),
            headerContrastSpread: contrastSpread(from: headerHist),
            bodyContrastSpread: contrastSpread(from: bodyHist),
            topSafeWarmFraction: topSafeRows > 0 ? Double(topSafeWarm) / Double(max(1, topSafeOpaque)) : nil,
            bottomSafeWarmFraction: bottomSafeRows > 0 ? Double(bottomSafeWarm) / Double(max(1, bottomSafeOpaque)) : nil
        )
    }

    static func maybeSaveArtifact(image: UIImage, name: String) {
        guard let dirPath = ProcessInfo.processInfo.environment["NOLA_VISUAL_ARTIFACT_DIR"],
              !dirPath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              let data = image.pngData() else {
            return
        }
        let dirURL = URL(fileURLWithPath: dirPath, isDirectory: true)
        try? FileManager.default.createDirectory(at: dirURL, withIntermediateDirectories: true)
        let sanitized = name.replacingOccurrences(of: "/", with: "-")
            .replacingOccurrences(of: " ", with: "_")
        try? data.write(to: dirURL.appendingPathComponent("\(sanitized).png"), options: .atomic)
    }

    private static func isWarmSurfacePixel(r: Int, g: Int, b: Int, targets: [RGBPixel], tolerance: Int) -> Bool {
        targets.contains { target in
            abs(r - target.r) <= tolerance &&
                abs(g - target.g) <= tolerance &&
                abs(b - target.b) <= tolerance
        }
    }

    private static func contrastSpread(from histogram: [Int]) -> Int {
        let total = histogram.reduce(0, +)
        guard total > 0 else { return 0 }
        let p01 = percentile(histogram: histogram, total: total, fraction: 0.0015)
        let p50 = percentile(histogram: histogram, total: total, fraction: 0.50)
        let p99 = percentile(histogram: histogram, total: total, fraction: 0.9985)
        return max(p99 - p50, p50 - p01)
    }

    private static func percentile(histogram: [Int], total: Int, fraction: Double) -> Int {
        let target = max(1, Int((Double(total) * fraction).rounded(.up)))
        var running = 0
        for bucket in 0..<256 {
            running += histogram[bucket]
            if running >= target {
                return bucket
            }
        }
        return 255
    }

    private static func formatPct(_ value: Double) -> String {
        String(format: "%.2f%%", value * 100)
    }
}
