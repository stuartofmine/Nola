// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelTypes logic in the iOS LLM integration layer.
// - Interfaces:
//   - LMModel
//   - SummaryItem
//   - ModelOutput
//   - ModelInput
// - APIs:
//   - none
//
//
//  ModelTypes.swift
//  Nola
//
//  Defines shared model types for LLM interactions.
//

import Foundation
#if canImport(MLXLMCommon) && !os(macOS)
import MLXLMCommon
#endif

// MARK: - Language Model Configuration

/// Represents a language model configuration with its associated properties and type.
#if canImport(MLXLMCommon) && !os(macOS)
public struct LMModel {
    /// Name of the model
    let name: String

    /// Configuration settings for model initialization
    let configuration: ModelConfiguration

    /// Type of the model (language or vision-language)
    let type: ModelType

    /// Defines the type of language model
    enum ModelType {
        /// Large language model (text-only)
        case llm
        /// Vision-language model (supports images and text)
        case vlm
    }
    
    /// Display name for the model
    var displayName: String {
        if isVisionModel {
            "\(name) (Vision)"
        } else {
            name
        }
    }

    /// Whether the model is a large language model
    var isLanguageModel: Bool {
        type == .llm
    }

    /// Whether the model is a vision-language model
    var isVisionModel: Bool {
        type == .vlm
    }
}

extension LMModel: Identifiable, Hashable {
    public var id: String {
        name
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(name)
    }
}
#endif

// MARK: - Model Output

nonisolated public struct SummaryItem: Codable, Hashable {
    public let text: String
    public let priority: Double
    public let dedupeKey: String
    public let action: String?
    
    public init(text: String, priority: Double, dedupeKey: String, action: String? = nil) {
        self.text = text
        self.priority = priority
        self.dedupeKey = dedupeKey
        self.action = action
    }
}

nonisolated public struct ModelOutput: Codable, Hashable {
    public let items: [SummaryItem]
    public let explain: String?

    public static let empty = ModelOutput(items: [], explain: nil)
}

// MARK: - Model Input

nonisolated public struct ModelInput: Codable, Hashable {
    public struct DateRange: Codable, Hashable { public let start: String; public let end: String }
    public struct Candidate: Codable, Hashable {
        public let type: String
        public let title: String?
        public let name: String?
        public let date: String?
        public let start: String?
        public let due: String?
        public let relation: String?
        public let notesTag: String?
        public let durationMin: Int?
    }
    public struct Attachment: Codable, Hashable {
        public let name: String
        public let mimeType: String
        public let base64Data: String

        public init(name: String, mimeType: String, base64Data: String) {
            self.name = name
            self.mimeType = mimeType
            self.base64Data = base64Data
        }
    }
    public let dateRange: DateRange
    public let candidates: [Candidate]
    public let userPrefs: [String: String]

    /// Binary attachments for providers that support multimodal chat input.
    /// Text extracted from documents still travels in `userPrefs["message"]` so
    /// OpenAI-compatible providers without file APIs can use it.
    public let attachments: [Attachment]
    public let providerContinuationJSON: String?

    private enum CodingKeys: String, CodingKey {
        case dateRange, candidates, userPrefs, attachments, providerContinuationJSON
    }

    public init(
        dateRange: DateRange,
        candidates: [Candidate],
        userPrefs: [String: String],
        attachments: [Attachment] = [],
        providerContinuationJSON: String? = nil
    ) {
        self.dateRange = dateRange
        self.candidates = candidates
        self.userPrefs = userPrefs
        self.attachments = attachments
        self.providerContinuationJSON = providerContinuationJSON
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.dateRange = try container.decode(DateRange.self, forKey: .dateRange)
        self.candidates = try container.decode([Candidate].self, forKey: .candidates)
        self.userPrefs = try container.decode([String: String].self, forKey: .userPrefs)
        self.attachments = try container.decodeIfPresent([Attachment].self, forKey: .attachments) ?? []
        self.providerContinuationJSON = try container.decodeIfPresent(String.self, forKey: .providerContinuationJSON)
    }
}
