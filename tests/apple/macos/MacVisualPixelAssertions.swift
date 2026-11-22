// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Hosts SwiftUI views in real titled NSWindows and validates theme sRGB pixel invariants and foreground contrast.

import AppKit
import CoreGraphics
import SwiftUI
import XCTest
@testable import Nola

@MainActor
enum MacVisualPixelAssertions {
    struct Metrics {
        let width: Int
        let height: Int
        let analyzedPixels: Int
        let warmSurfaceFraction: Double
        let pureBlackFraction: Double
        let fullWidthBlackScanlines: Int
        let coolGrayFraction: Double
        let headerContrastDelta: Double
        let bodyContrastDelta: Double
    }

    private struct RGB: Equatable {
        let r: Int
        let g: Int
        let b: Int

        init(_ value: UInt32) {
            r = Int((value >> 16) & 0xFF)
            g = Int((value >> 8) & 0xFF)
            b = Int(value & 0xFF)
        }
    }

    private static let lightBackgrounds = [
        MacDesignTokens.Palette.Light.canvas,
        MacDesignTokens.Palette.Light.surface,
        MacDesignTokens.Palette.Light.surfaceSubtle,
    ].map(RGB.init)

    private static let darkBackgrounds = [
        MacDesignTokens.Palette.Dark.canvas,
        MacDesignTokens.Palette.Dark.surface,
        MacDesignTokens.Palette.Dark.surfaceSubtle,
    ].map(RGB.init)

    @discardableResult
    static func assertView<Content: View>(
        _ content: Content,
        name: String,
        size: CGSize,
        appearance: NSAppearance.Name,
        title: String = "Nola",
        styleMask: NSWindow.StyleMask = [.titled, .closable, .miniaturizable, .resizable],
        configureWindow: ((NSWindow) -> Void)? = nil,
        beforeCapture: ((NSWindow, NSView) async throws -> Void)? = nil,
        assertWindow: ((NSWindow, NSView) async throws -> Void)? = nil,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws -> Metrics {
        let isDark = appearance == .darkAqua || appearance == .vibrantDark
        let nsAppearance = try XCTUnwrap(NSAppearance(named: appearance), file: file, line: line)
        let previousAppAppearance = NSApplication.shared.appearance
        NSApplication.shared.appearance = nsAppearance
        defer { NSApplication.shared.appearance = previousAppAppearance }

        let wrapped = content
            .frame(width: size.width, height: size.height)
            .environment(\.colorScheme, isDark ? .dark : .light)
        let hostingView = NSHostingView(rootView: wrapped)
        hostingView.frame = NSRect(origin: .zero, size: size)
        hostingView.appearance = nsAppearance

        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: size),
            styleMask: styleMask,
            backing: .buffered,
            defer: false
        )
        window.title = title
        window.appearance = nsAppearance
        window.animationBehavior = .none
        window.isReleasedWhenClosed = false
        window.contentView = hostingView
        configureWindow?(window)
        window.orderFrontRegardless()
        defer {
            window.orderOut(nil)
            window.contentView = nil
            window.close()
        }

        try await settleLayout(window: window, hostingView: hostingView)
        if let beforeCapture {
            try await beforeCapture(window, hostingView)
            try await settleLayout(window: window, hostingView: hostingView)
        }
        if let assertWindow {
            try await assertWindow(window, hostingView)
        }

        let captureTarget = window.contentView?.superview ?? hostingView
        let cgImage = try captureCGImage(of: captureTarget)
        let contentRect = contentPixelRect(
            hostingView: hostingView,
            in: captureTarget,
            pixelWidth: cgImage.width,
            pixelHeight: cgImage.height
        )

        let metrics = try analyzePixels(
            cgImage: cgImage,
            contentRect: contentRect,
            isDark: isDark
        )
        assertMetrics(metrics, name: name, isDark: isDark, file: file, line: line)
        try writeDebugArtifactIfConfigured(cgImage: cgImage, view: hostingView, name: name)
        return metrics
    }

    @discardableResult
    static func assertSheet<SheetContent: View>(
        _ sheetContent: SheetContent,
        name: String,
        parentSize: CGSize = CGSize(width: 960, height: 700),
        sheetSize: CGSize,
        appearance: NSAppearance.Name,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws -> Metrics {
        let isDark = appearance == .darkAqua || appearance == .vibrantDark
        let nsAppearance = try XCTUnwrap(NSAppearance(named: appearance), file: file, line: line)
        let previousAppAppearance = NSApplication.shared.appearance
        NSApplication.shared.appearance = nsAppearance
        defer { NSApplication.shared.appearance = previousAppAppearance }

        let parentHost = NSHostingView(
            rootView: MacDesignColor.canvas
                .frame(width: parentSize.width, height: parentSize.height)
                .environment(\.colorScheme, isDark ? .dark : .light)
        )
        parentHost.frame = NSRect(origin: .zero, size: parentSize)
        parentHost.appearance = nsAppearance

        let parentWindow = NSWindow(
            contentRect: NSRect(origin: .zero, size: parentSize),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        parentWindow.appearance = nsAppearance
        parentWindow.animationBehavior = .none
        parentWindow.isReleasedWhenClosed = false
        parentWindow.backgroundColor = NSColor(named: "NolaCanvas")
        parentWindow.contentView = parentHost
        parentWindow.orderFrontRegardless()
        defer {
            parentWindow.orderOut(nil)
            parentWindow.contentView = nil
            parentWindow.close()
        }

        let sheetHost = NSHostingView(
            rootView: sheetContent
                .frame(minWidth: sheetSize.width, minHeight: sheetSize.height)
                .background(MacDesignColor.canvas)
                .environment(\.colorScheme, isDark ? .dark : .light)
        )
        let resolvedSize = CGSize(
            width: max(sheetSize.width, ceil(sheetHost.fittingSize.width)),
            height: max(sheetSize.height, ceil(sheetHost.fittingSize.height))
        )
        sheetHost.frame = NSRect(origin: .zero, size: resolvedSize)
        sheetHost.appearance = nsAppearance

        let sheetWindow = NSWindow(
            contentRect: NSRect(origin: .zero, size: resolvedSize),
            styleMask: [.titled],
            backing: .buffered,
            defer: false
        )
        sheetWindow.appearance = nsAppearance
        sheetWindow.animationBehavior = .none
        sheetWindow.isReleasedWhenClosed = false
        sheetWindow.backgroundColor = NSColor(named: "NolaCanvas")
        sheetWindow.contentView = sheetHost

        parentWindow.beginSheet(sheetWindow, completionHandler: nil)
        defer {
            if parentWindow.attachedSheet != nil {
                parentWindow.endSheet(sheetWindow)
            }
            sheetWindow.orderOut(nil)
            sheetWindow.contentView = nil
            sheetWindow.close()
        }

        let deadline = ContinuousClock.now + .seconds(2)
        while parentWindow.attachedSheet == nil, ContinuousClock.now < deadline {
            await Task.yield()
            try await Task.sleep(for: .milliseconds(25))
        }
        XCTAssertNotNil(parentWindow.attachedSheet, "\(name) failed to attach sheet", file: file, line: line)

        try await settleLayout(window: sheetWindow, hostingView: sheetHost)
        let captureTarget = sheetWindow.contentView?.superview ?? sheetHost
        let cgImage = try captureCGImage(of: captureTarget)
        let contentRect = contentPixelRect(
            hostingView: sheetHost,
            in: captureTarget,
            pixelWidth: cgImage.width,
            pixelHeight: cgImage.height
        )
        let metrics = try analyzePixels(
            cgImage: cgImage,
            contentRect: contentRect,
            isDark: isDark
        )
        assertMetrics(metrics, name: name, isDark: isDark, file: file, line: line)
        try writeDebugArtifactIfConfigured(cgImage: cgImage, view: sheetHost, name: name)
        return metrics
    }

    static func settleLayout(window: NSWindow, hostingView: NSView) async throws {
        await Task.yield()
        hostingView.layoutSubtreeIfNeeded()
        window.layoutIfNeeded()
        try await Task.sleep(for: .milliseconds(320))
        await Task.yield()
        hostingView.layoutSubtreeIfNeeded()
        window.contentView?.superview?.layoutSubtreeIfNeeded()
        window.layoutIfNeeded()
    }

    static func captureCGImage(of view: NSView) throws -> CGImage {
        view.layoutSubtreeIfNeeded()
        let rep = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        rep.size = view.bounds.size
        view.cacheDisplay(in: view.bounds, to: rep)
        let sourceImage = try XCTUnwrap(rep.cgImage)

        let width = sourceImage.width
        let height = sourceImage.height
        let sRGB = try XCTUnwrap(CGColorSpace(name: CGColorSpace.sRGB))
        let context = try XCTUnwrap(CGContext(
            data: nil,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: width * 4,
            space: sRGB,
            bitmapInfo: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue
        ))
        context.draw(sourceImage, in: CGRect(x: 0, y: 0, width: width, height: height))
        return try XCTUnwrap(context.makeImage())
    }

    private static func contentPixelRect(
        hostingView: NSView,
        in captureTarget: NSView,
        pixelWidth: Int,
        pixelHeight: Int
    ) -> CGRect {
        let targetBounds = captureTarget.bounds
        guard targetBounds.width > 0, targetBounds.height > 0 else {
            return CGRect(x: 0, y: 0, width: pixelWidth, height: pixelHeight)
        }
        let converted = hostingView.convert(hostingView.bounds, to: captureTarget)
        let scaleX = CGFloat(pixelWidth) / targetBounds.width
        let scaleY = CGFloat(pixelHeight) / targetBounds.height

        let minX = max(0, Int(floor(converted.minX * scaleX)))
        let maxX = min(pixelWidth, Int(ceil(converted.maxX * scaleX)))
        let topPt = captureTarget.isFlipped ? converted.minY : (targetBounds.height - converted.maxY)
        let bottomPt = captureTarget.isFlipped ? converted.maxY : (targetBounds.height - converted.minY)
        let minY = max(0, Int(floor(topPt * scaleY)))
        let maxY = min(pixelHeight, Int(ceil(bottomPt * scaleY)))
        return CGRect(x: minX, y: minY, width: max(1, maxX - minX), height: max(1, maxY - minY))
    }

    static func analyzePixels(
        cgImage: CGImage,
        contentRect: CGRect,
        isDark: Bool
    ) throws -> Metrics {
        let width = cgImage.width
        let height = cgImage.height
        let sRGB = try XCTUnwrap(CGColorSpace(name: CGColorSpace.sRGB))
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        try pixels.withUnsafeMutableBytes { rawBuffer in
            let context = try XCTUnwrap(CGContext(
                data: rawBuffer.baseAddress,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: width * 4,
                space: sRGB,
                bitmapInfo: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue
            ))
            context.draw(cgImage, in: CGRect(x: 0, y: 0, width: width, height: height))
        }

        let x0 = max(0, Int(contentRect.minX))
        let x1 = min(width, Int(contentRect.maxX))
        let y0 = max(0, Int(contentRect.minY))
        let y1 = min(height, Int(contentRect.maxY))
        let splitY = y0 + max(1, Int(Double(y1 - y0) * 0.30))

        let allowedBackgrounds = isDark ? darkBackgrounds : lightBackgrounds
        let selection = RGB(isDark ? MacDesignTokens.Palette.Dark.accentSubtle : MacDesignTokens.Palette.Light.accentSubtle)
        var analyzed = 0
        var warmSurfaceCount = 0
        var pureBlackCount = 0
        var fullWidthBlackScanlines = 0
        var coolGrayCount = 0
        var headerLuminanceCounts = [Int](repeating: 0, count: 256)
        var bodyLuminanceCounts = [Int](repeating: 0, count: 256)
        var headerSamples = 0
        var bodySamples = 0

        for y in y0..<y1 {
            var rowOpaque = 0
            var rowBlack = 0
            for x in x0..<x1 {
                let idx = (y * width + x) * 4
                let a = Int(pixels[idx + 3])
                guard a >= 245 else { continue }
                let r = Int(pixels[idx])
                let g = Int(pixels[idx + 1])
                let b = Int(pixels[idx + 2])

                analyzed += 1
                rowOpaque += 1

                if allowedBackgrounds.contains(where: {
                    abs(r - $0.r) <= 6 && abs(g - $0.g) <= 6 && abs(b - $0.b) <= 6
                }) {
                    warmSurfaceCount += 1
                }

                if r <= 3 && g <= 3 && b <= 3 {
                    pureBlackCount += 1
                    rowBlack += 1
                }

                // The selected-row fill deliberately has a slight blue cast in both themes.
                let selectedFill = r == selection.r && g == selection.g && b == selection.b
                let blueCast = !selectedFill && b >= g && b - r >= 4
                let isLightCoolGray = !isDark && r > 215 && blueCast
                let isDarkCoolGray = isDark && (20...55).contains(r) && blueCast
                if isLightCoolGray || isDarkCoolGray {
                    coolGrayCount += 1
                }

                let lum = min(255, max(0, Int((0.2126 * Double(r) + 0.7152 * Double(g) + 0.0722 * Double(b)).rounded())))
                if y < splitY {
                    headerLuminanceCounts[lum] += 1
                    headerSamples += 1
                } else {
                    bodyLuminanceCounts[lum] += 1
                    bodySamples += 1
                }
            }
            if rowOpaque > 0 && Double(rowBlack) / Double(rowOpaque) >= 0.95 {
                fullWidthBlackScanlines += 1
            }
        }

        let totalRectPixels = max(1, (x1 - x0) * (y1 - y0))
        XCTAssertGreaterThan(Double(analyzed) / Double(totalRectPixels), 0.90, "Window content area must be opaque")

        let headerDelta = regionalContrastDelta(counts: headerLuminanceCounts, total: headerSamples)
        let bodyDelta = regionalContrastDelta(counts: bodyLuminanceCounts, total: bodySamples)

        return Metrics(
            width: width,
            height: height,
            analyzedPixels: max(1, analyzed),
            warmSurfaceFraction: Double(warmSurfaceCount) / Double(max(1, analyzed)),
            pureBlackFraction: Double(pureBlackCount) / Double(max(1, analyzed)),
            fullWidthBlackScanlines: fullWidthBlackScanlines,
            coolGrayFraction: Double(coolGrayCount) / Double(max(1, analyzed)),
            headerContrastDelta: headerDelta,
            bodyContrastDelta: bodyDelta
        )
    }

    private static func regionalContrastDelta(counts: [Int], total: Int) -> Double {
        guard total > 0 else { return 0 }
        func percentile(_ fraction: Double) -> Double {
            let target = max(1, Int(ceil(Double(total) * fraction)))
            var cumulative = 0
            for lum in 0..<256 {
                cumulative += counts[lum]
                if cumulative >= target {
                    return Double(lum)
                }
            }
            return 255
        }
        let median = percentile(0.50)
        let high = percentile(0.997)
        let low = percentile(0.003)
        return max(high - median, median - low)
    }

    private static func assertMetrics(
        _ metrics: Metrics,
        name: String,
        isDark: Bool,
        file: StaticString,
        line: UInt
    ) {
        XCTAssertGreaterThanOrEqual(
            metrics.warmSurfaceFraction,
            0.35,
            "\(name) dominant background does not match the \(isDark ? "dark" : "light") surface palette (matched \(String(format: "%.2f%%", metrics.warmSurfaceFraction * 100)))",
            file: file,
            line: line
        )
        if isDark {
            XCTAssertLessThan(
                metrics.pureBlackFraction,
                0.005,
                "\(name) leaked pure-black #000000 pixels (\(String(format: "%.2f%%", metrics.pureBlackFraction * 100)))",
                file: file,
                line: line
            )
            XCTAssertEqual(
                metrics.fullWidthBlackScanlines,
                0,
                "\(name) contains \(metrics.fullWidthBlackScanlines) full-width black scanline(s)",
                file: file,
                line: line
            )
        }
        XCTAssertLessThan(
            metrics.coolGrayFraction,
            0.01,
            "\(name) leaked cool system gray pixels (\(String(format: "%.2f%%", metrics.coolGrayFraction * 100)))",
            file: file,
            line: line
        )
        XCTAssertGreaterThanOrEqual(
            max(metrics.headerContrastDelta, metrics.bodyContrastDelta),
            40.0,
            "\(name) rendered blank or low-contrast content (headerΔ=\(metrics.headerContrastDelta), bodyΔ=\(metrics.bodyContrastDelta))",
            file: file,
            line: line
        )
    }

    private static func writeDebugArtifactIfConfigured(cgImage: CGImage, view: NSView, name: String) throws {
        guard let directory = ProcessInfo.processInfo.environment["NOLA_VISUAL_ARTIFACT_DIR"],
              !directory.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        let outputDir = URL(fileURLWithPath: directory, isDirectory: true)
        try FileManager.default.createDirectory(at: outputDir, withIntermediateDirectories: true)
        let rep = NSBitmapImageRep(cgImage: cgImage)
        let png = try XCTUnwrap(rep.representation(using: .png, properties: [:]))
        try png.write(to: outputDir.appendingPathComponent(name + ".png"), options: .atomic)
        let session = try MacHostedAccessibilitySession()
        defer { session.restore() }
        let controls = session.visibleControls(in: view)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try encoder.encode(controls).write(to: outputDir.appendingPathComponent(name + "-controls.json"), options: .atomic)
    }
}
