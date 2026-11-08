#!/usr/bin/env swift

import AppKit
import CoreText
import Foundation
import PDFKit

private struct ReportSection: Decodable {
    let heading: String
    let paragraphs: [String]
}

private struct ReportSpec: Decodable {
    let title: String
    let summary: String?
    let sections: [ReportSection]
    let footer: String?
}

private enum ReportError: Error, LocalizedError {
    case missingArgument(String)
    case invalidSpec(String)
    case cannotCreatePDF
    case invalidPDF

    var errorDescription: String? {
        switch self {
        case .missingArgument(let value): return "Missing required argument: \(value)"
        case .invalidSpec(let value): return value
        case .cannotCreatePDF: return "Unable to create the PDF context."
        case .invalidPDF: return "The generated file is not a readable PDF."
        }
    }
}

private let pageSize = CGSize(width: 612, height: 792)
private let contentRect = CGRect(x: 54, y: 82, width: 504, height: 632)
private let accentColor = NSColor(calibratedRed: 23 / 255, green: 105 / 255, blue: 224 / 255, alpha: 1)
private let textColor = NSColor(calibratedRed: 24 / 255, green: 33 / 255, blue: 43 / 255, alpha: 1)
private let secondaryColor = NSColor(calibratedRed: 83 / 255, green: 99 / 255, blue: 117 / 255, alpha: 1)

private func paragraphStyle(lineSpacing: CGFloat, paragraphSpacing: CGFloat) -> NSParagraphStyle {
    let style = NSMutableParagraphStyle()
    style.lineSpacing = lineSpacing
    style.paragraphSpacing = paragraphSpacing
    return style
}

private func append(
    _ text: String,
    to output: NSMutableAttributedString,
    font: NSFont,
    color: NSColor,
    lineSpacing: CGFloat,
    paragraphSpacing: CGFloat
) {
    output.append(
        NSAttributedString(
            string: text,
            attributes: [
                .font: font,
                .foregroundColor: color,
                .paragraphStyle: paragraphStyle(
                    lineSpacing: lineSpacing,
                    paragraphSpacing: paragraphSpacing
                ),
            ]
        )
    )
}

private func reportText(_ spec: ReportSpec) throws -> NSAttributedString {
    let title = spec.title.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !title.isEmpty else {
        throw ReportError.invalidSpec("PDF report title cannot be empty.")
    }
    guard !spec.sections.isEmpty else {
        throw ReportError.invalidSpec("PDF report must contain at least one section.")
    }

    let output = NSMutableAttributedString()
    append(
        title + "\n",
        to: output,
        font: .systemFont(ofSize: 25, weight: .bold),
        color: accentColor,
        lineSpacing: 3,
        paragraphSpacing: 14
    )
    if let summary = spec.summary?.trimmingCharacters(in: .whitespacesAndNewlines),
       !summary.isEmpty {
        append(
            summary + "\n",
            to: output,
            font: .systemFont(ofSize: 12.5, weight: .medium),
            color: secondaryColor,
            lineSpacing: 4,
            paragraphSpacing: 18
        )
    }
    for (index, section) in spec.sections.enumerated() {
        let heading = section.heading.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !heading.isEmpty, !section.paragraphs.isEmpty else {
            throw ReportError.invalidSpec("Every PDF section needs a heading and content.")
        }
        append(
            String(format: "%02d  %@\n", index + 1, heading),
            to: output,
            font: .systemFont(ofSize: 16, weight: .semibold),
            color: textColor,
            lineSpacing: 3,
            paragraphSpacing: 8
        )
        for paragraph in section.paragraphs {
            append(
                paragraph.trimmingCharacters(in: .whitespacesAndNewlines) + "\n",
                to: output,
                font: .systemFont(ofSize: 11.5, weight: .regular),
                color: textColor,
                lineSpacing: 4,
                paragraphSpacing: 9
            )
        }
    }
    return output
}

private func drawLine(
    _ text: String,
    at point: CGPoint,
    fontSize: CGFloat,
    color: NSColor,
    in context: CGContext
) {
    let attributed = NSAttributedString(
        string: text,
        attributes: [
            .font: NSFont.systemFont(ofSize: fontSize, weight: .medium),
            .foregroundColor: color,
        ]
    )
    let line = CTLineCreateWithAttributedString(attributed)
    context.textPosition = point
    CTLineDraw(line, context)
}

private func createPDF(spec: ReportSpec, outputURL: URL) throws {
    let content = try reportText(spec)
    try FileManager.default.createDirectory(
        at: outputURL.deletingLastPathComponent(),
        withIntermediateDirectories: true
    )
    var mediaBox = CGRect(origin: .zero, size: pageSize)
    guard let context = CGContext(outputURL as CFURL, mediaBox: &mediaBox, nil) else {
        throw ReportError.cannotCreatePDF
    }
    let framesetter = CTFramesetterCreateWithAttributedString(content)
    var location = 0
    var pageNumber = 1

    while location < content.length {
        context.beginPDFPage(nil)
        context.setFillColor(NSColor.white.cgColor)
        context.fill(mediaBox)
        context.setFillColor(accentColor.cgColor)
        context.fill(CGRect(x: 54, y: 741, width: 64, height: 5))
        context.setStrokeColor(NSColor(calibratedWhite: 0.86, alpha: 1).cgColor)
        context.setLineWidth(0.6)
        context.move(to: CGPoint(x: 54, y: 54))
        context.addLine(to: CGPoint(x: 558, y: 54))
        context.strokePath()

        let path = CGPath(rect: contentRect, transform: nil)
        let frame = CTFramesetterCreateFrame(
            framesetter,
            CFRange(location: location, length: 0),
            path,
            nil
        )
        CTFrameDraw(frame, context)
        let visible = CTFrameGetVisibleStringRange(frame)
        guard visible.length > 0 else {
            throw ReportError.invalidSpec("PDF content could not be paginated.")
        }
        location += visible.length

        drawLine(
            spec.footer ?? "Generated locally by Nola",
            at: CGPoint(x: 54, y: 34),
            fontSize: 8.5,
            color: secondaryColor,
            in: context
        )
        drawLine(
            String(format: "%02d", pageNumber),
            at: CGPoint(x: 531, y: 34),
            fontSize: 8.5,
            color: secondaryColor,
            in: context
        )
        context.endPDFPage()
        pageNumber += 1
    }
    context.closePDF()
}

private func validatePDF(_ outputURL: URL) throws -> [String: Any] {
    let data = try Data(contentsOf: outputURL)
    guard data.starts(with: Data("%PDF".utf8)),
          let document = PDFDocument(url: outputURL),
          document.pageCount > 0,
          let text = document.string,
          !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
        throw ReportError.invalidPDF
    }
    return [
        "bytes": data.count,
        "characters": text.count,
        "pages": document.pageCount,
        "validated": true,
    ]
}

private func argument(_ name: String) -> String? {
    guard let index = CommandLine.arguments.firstIndex(of: name),
          CommandLine.arguments.indices.contains(index + 1) else {
        return nil
    }
    return CommandLine.arguments[index + 1]
}

do {
    guard let outputPath = argument("--output") else {
        throw ReportError.missingArgument("--output")
    }
    let outputURL = URL(fileURLWithPath: outputPath)
    if !CommandLine.arguments.contains("--validate-only") {
        guard let specPath = argument("--spec") else {
            throw ReportError.missingArgument("--spec")
        }
        let data = try Data(contentsOf: URL(fileURLWithPath: specPath))
        let spec = try JSONDecoder().decode(ReportSpec.self, from: data)
        try createPDF(spec: spec, outputURL: outputURL)
    }
    let receipt = try validatePDF(outputURL)
    let receiptData = try JSONSerialization.data(withJSONObject: receipt, options: [.sortedKeys])
    print(String(decoding: receiptData, as: UTF8.self))
} catch {
    FileHandle.standardError.write(Data("PDF report failed: \(error.localizedDescription)\n".utf8))
    exit(1)
}
