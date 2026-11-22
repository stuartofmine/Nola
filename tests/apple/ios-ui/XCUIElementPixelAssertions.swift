// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: sRGB pixel, Warm Paper/Ink theme, leak, and contrast verification for XCUIScreenshot captures in NolaUITests.

import CoreGraphics
import Foundation
import UIKit
import XCTest

struct XCUIElementPixelSummary {
    let width: Int
    let height: Int
    let warmSurfaceFraction: Double
    let pureBlackFraction: Double
    let fullWidthBlackScanlines: Int
    let coolGrayFraction: Double
    let foregroundContrastSpread: Int
}

enum XCUIElementPixelAssertions {
    private struct RGBPixel {
        let r: Int
        let g: Int
        let b: Int
    }

    private static let lightSurfaceTargets: [RGBPixel] = [
        RGBPixel(r: 250, g: 248, b: 244), // #FAFAFA
        RGBPixel(r: 255, g: 255, b: 255), // #FFFFFF
        RGBPixel(r: 241, g: 239, b: 232)  // #F2F2F4
    ]

    private static let darkSurfaceTargets: [RGBPixel] = [
        RGBPixel(r: 27, g: 28, b: 26), // #0b0b0c
        RGBPixel(r: 34, g: 36, b: 32), // #131315
        RGBPixel(r: 41, g: 44, b: 39)  // #1a1a1d
    ]

    @discardableResult
    static func assertElementScreenshot(
        _ element: XCUIElement,
        label: String,
        isDark: Bool,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws -> XCUIElementPixelSummary {
        try assertScreenshot(element.screenshot(), label: label, isDark: isDark, file: file, line: line)
    }

    @discardableResult
    static func assertScreenshot(
        _ screenshot: XCUIScreenshot,
        label: String,
        isDark: Bool,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws -> XCUIElementPixelSummary {
        let cgImage = try XCTUnwrap(screenshot.image.cgImage, "Missing CGImage for screenshot \(label)", file: file, line: line)
        maybeSaveArtifact(screenshot: screenshot, name: label)
        let summary = try analyze(cgImage: cgImage, isDark: isDark)

        XCTAssertGreaterThanOrEqual(
            summary.warmSurfaceFraction, 0.30,
            "\(label): dominant surface must match Warm \(isDark ? "Ink" : "Paper") (warm=\(formatPct(summary.warmSurfaceFraction)))",
            file: file, line: line
        )
        if isDark {
            XCTAssertLessThan(
                summary.pureBlackFraction, 0.01,
                "\(label): detected OLED #000000 background leak (\(formatPct(summary.pureBlackFraction)))",
                file: file, line: line
            )
            XCTAssertEqual(
                summary.fullWidthBlackScanlines, 0,
                "\(label): detected \(summary.fullWidthBlackScanlines) full-width black scanline(s)",
                file: file, line: line
            )
        }
        XCTAssertLessThan(
            summary.coolGrayFraction, 0.03,
            "\(label): detected UIKit cool grouped gray leak such as #F2F2F7/#1C1C1E (\(formatPct(summary.coolGrayFraction)))",
            file: file, line: line
        )
        XCTAssertGreaterThanOrEqual(
            summary.foregroundContrastSpread, 40,
            "\(label): screenshot lacks visible foreground contrast (spread=\(summary.foregroundContrastSpread)/255)",
            file: file, line: line
        )
        return summary
    }

    static func analyze(cgImage: CGImage, isDark: Bool) throws -> XCUIElementPixelSummary {
        let width = cgImage.width
        let height = cgImage.height
        guard width > 0, height > 0 else {
            throw NSError(domain: "XCUIElementPixelAssertions", code: 1, userInfo: [NSLocalizedDescriptionKey: "Empty CGImage"])
        }
        guard let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) else {
            throw NSError(domain: "XCUIElementPixelAssertions", code: 2, userInfo: [NSLocalizedDescriptionKey: "Unavailable sRGB CGColorSpace"])
        }

        var buffer = [UInt8](repeating: 0, count: width * height * 4)
        let bitmapInfo = CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue
        let drew = buffer.withUnsafeMutableBytes { raw -> Bool in
            guard let ctx = CGContext(
                data: raw.baseAddress,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: width * 4,
                space: colorSpace,
                bitmapInfo: bitmapInfo
            ) else {
                return false
            }
            ctx.draw(cgImage, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        guard drew else {
            throw NSError(domain: "XCUIElementPixelAssertions", code: 3, userInfo: [NSLocalizedDescriptionKey: "Failed to rasterize CGImage"])
        }

        // Skip top status bar capsule rows so hardware notch/island cutouts do not trigger false scanlines.
        let contentStartRow = min(height / 14, max(0, Int(Double(height) * 0.05)))
        let targets = isDark ? darkSurfaceTargets : lightSurfaceTargets

        var analyzedCount = 0
        var warmSurfaceCount = 0
        var pureBlackCount = 0
        var fullWidthBlackScanlines = 0
        var coolGrayCount = 0
        var histogram = [Int](repeating: 0, count: 256)

        for y in contentStartRow..<height {
            var rowBlackCount = 0
            for x in 0..<width {
                let offset = (y * width + x) * 4
                let r = Int(buffer[offset])
                let g = Int(buffer[offset + 1])
                let b = Int(buffer[offset + 2])

                analyzedCount += 1
                if targets.contains(where: { abs(r - $0.r) <= 8 && abs(g - $0.g) <= 8 && abs(b - $0.b) <= 8 }) {
                    warmSurfaceCount += 1
                }
                if r <= 3 && g <= 3 && b <= 3 {
                    pureBlackCount += 1
                    rowBlackCount += 1
                }
                let isNeutralCoolGray: Bool
                if isDark {
                    isNeutralCoolGray = abs(r - g) <= 2 && (
                        ((22...34).contains(r) && (b - r) >= 2) ||
                        ((35...55).contains(r) && (b - r) >= 3)
                    )
                } else {
                    isNeutralCoolGray = r > 220 && (b - r) >= 3 && abs(r - g) <= 4
                }
                if isNeutralCoolGray {
                    coolGrayCount += 1
                }

                let luma = min(255, max(0, (299 * r + 587 * g + 114 * b) / 1000))
                histogram[luma] += 1
            }
            if rowBlackCount == width {
                fullWidthBlackScanlines += 1
            }
        }

        let safeDenom = max(1, analyzedCount)
        return XCUIElementPixelSummary(
            width: width,
            height: height,
            warmSurfaceFraction: Double(warmSurfaceCount) / Double(safeDenom),
            pureBlackFraction: Double(pureBlackCount) / Double(safeDenom),
            fullWidthBlackScanlines: fullWidthBlackScanlines,
            coolGrayFraction: Double(coolGrayCount) / Double(safeDenom),
            foregroundContrastSpread: contrastSpread(from: histogram)
        )
    }

    private static func maybeSaveArtifact(screenshot: XCUIScreenshot, name: String) {
        guard let dirPath = ProcessInfo.processInfo.environment["NOLA_VISUAL_ARTIFACT_DIR"],
              !dirPath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        let dirURL = URL(fileURLWithPath: dirPath, isDirectory: true)
        try? FileManager.default.createDirectory(at: dirURL, withIntermediateDirectories: true)
        let sanitized = name.replacingOccurrences(of: "/", with: "-")
            .replacingOccurrences(of: " ", with: "_")
        try? screenshot.pngRepresentation.write(to: dirURL.appendingPathComponent("\(sanitized).png"), options: .atomic)
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
