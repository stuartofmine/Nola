#!/usr/bin/env swift

import AppKit
import Foundation
import Vision

private struct Arguments {
    var input: URL?
    var output: URL?
    var textOutput: URL?
    var validateOnly = false
}

private enum OCRFailure: Error, LocalizedError {
    case missing(String)
    case invalid(String)

    var errorDescription: String? {
        switch self {
        case .missing(let value): return "Missing argument: \(value)"
        case .invalid(let value): return value
        }
    }
}

private func parseArguments() throws -> Arguments {
    var result = Arguments()
    var index = 1
    let values = CommandLine.arguments
    while index < values.count {
        switch values[index] {
        case "--input":
            index += 1
            guard index < values.count else { throw OCRFailure.missing("--input value") }
            result.input = URL(fileURLWithPath: values[index])
        case "--output":
            index += 1
            guard index < values.count else { throw OCRFailure.missing("--output value") }
            result.output = URL(fileURLWithPath: values[index])
        case "--text-output":
            index += 1
            guard index < values.count else { throw OCRFailure.missing("--text-output value") }
            result.textOutput = URL(fileURLWithPath: values[index])
        case "--validate-only": result.validateOnly = true
        default: throw OCRFailure.invalid("Unknown argument: \(values[index])")
        }
        index += 1
    }
    guard result.output != nil else { throw OCRFailure.missing("--output") }
    if !result.validateOnly, result.input == nil { throw OCRFailure.missing("--input") }
    return result
}

private func recognize(inputURL: URL) throws -> [[String: Any]] {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = false
    request.recognitionLanguages = ["en-US"]
    try VNImageRequestHandler(url: inputURL, options: [:]).perform([request])
    let results = (request.results ?? []).sorted { left, right in
        if abs(left.boundingBox.midY - right.boundingBox.midY) > 0.025 {
            return left.boundingBox.midY > right.boundingBox.midY
        }
        return left.boundingBox.minX < right.boundingBox.minX
    }
    return results.compactMap { observation in
        guard let candidate = observation.topCandidates(1).first else { return nil }
        let box = observation.boundingBox
        return [
            "text": candidate.string,
            "confidence": Double(candidate.confidence),
            "bounding_box": [
                "x": Double(box.origin.x), "y": Double(box.origin.y),
                "width": Double(box.width), "height": Double(box.height),
            ],
        ]
    }
}

private func createResult(inputURL: URL, outputURL: URL) throws {
    let observations = try recognize(inputURL: inputURL)
    guard !observations.isEmpty else { throw OCRFailure.invalid("Vision returned no text observations.") }
    let rawText = observations.compactMap { $0["text"] as? String }.joined(separator: "\n")
    let uncertain = observations.filter { ($0["confidence"] as? Double ?? 0) < 0.8 }.map {
        ["text": $0["text"] ?? "", "bounding_box": $0["bounding_box"] ?? [:]]
    }
    let object: [String: Any] = [
        "source": inputURL.path,
        "reading_order": "top-to-bottom, left-to-right within each line",
        "raw_text": rawText,
        "normalized_text": rawText,
        "normalization_decisions": [],
        "observations": observations,
        "minimum_confidence": observations.compactMap { $0["confidence"] as? Double }.min() ?? 0,
        "uncertain_regions": uncertain,
    ]
    try FileManager.default.createDirectory(at: outputURL.deletingLastPathComponent(), withIntermediateDirectories: true)
    try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys])
        .write(to: outputURL, options: .atomic)
}

private func writeTextOutput(resultURL: URL, textOutputURL: URL) throws {
    let data = try Data(contentsOf: resultURL)
    guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
          let rawText = object["raw_text"] as? String, !rawText.isEmpty else {
        throw OCRFailure.invalid("OCR JSON cannot produce a text output.")
    }
    let delivered = rawText.hasSuffix("\n") ? rawText : rawText + "\n"
    try FileManager.default.createDirectory(
        at: textOutputURL.deletingLastPathComponent(),
        withIntermediateDirectories: true
    )
    try delivered.write(to: textOutputURL, atomically: true, encoding: .utf8)
}

private func validate(outputURL: URL, textOutputURL: URL?) throws -> [String: Any] {
    let data = try Data(contentsOf: outputURL)
    guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
          let observations = object["observations"] as? [[String: Any]], !observations.isEmpty,
          let rawText = object["raw_text"] as? String, !rawText.isEmpty,
          object["normalized_text"] as? String == rawText,
          object["reading_order"] as? String != nil,
          object["uncertain_regions"] as? [[String: Any]] != nil else {
        throw OCRFailure.invalid("OCR output is missing its required evidence.")
    }
    for observation in observations {
        guard observation["text"] as? String != nil,
              let confidence = observation["confidence"] as? Double, (0...1).contains(confidence),
              observation["bounding_box"] as? [String: Any] != nil else {
            throw OCRFailure.invalid("OCR observation is incomplete.")
        }
    }
    var receipt: [String: Any] = [
        "bytes": data.count,
        "observations": observations.count,
        "text": rawText,
        "validated": true,
    ]
    if let textOutputURL {
        let delivered = try String(contentsOf: textOutputURL, encoding: .utf8)
        let expected = rawText.hasSuffix("\n") ? rawText : rawText + "\n"
        guard delivered == expected else {
            throw OCRFailure.invalid("Text output does not exactly match OCR reading order.")
        }
        receipt["text_bytes"] = delivered.lengthOfBytes(using: .utf8)
        receipt["text_output_validated"] = true
    }
    return receipt
}

do {
    let arguments = try parseArguments()
    if !arguments.validateOnly {
        try createResult(inputURL: arguments.input!, outputURL: arguments.output!)
        if let textOutput = arguments.textOutput {
            try writeTextOutput(resultURL: arguments.output!, textOutputURL: textOutput)
        }
    }
    let receipt = try validate(
        outputURL: arguments.output!,
        textOutputURL: arguments.textOutput
    )
    print(String(decoding: try JSONSerialization.data(withJSONObject: receipt, options: [.sortedKeys]), as: UTF8.self))
} catch {
    FileHandle.standardError.write(Data("OCR failed: \(error.localizedDescription)\n".utf8))
    exit(1)
}
