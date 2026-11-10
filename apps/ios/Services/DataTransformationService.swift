// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements DataTransformationService logic in the iOS service layer.
// - Interfaces:
//   - DataTransformationService
//   - DefaultDataTransformationService
// - APIs:
//   - none
//
import Foundation

// Transform LLM ModelOutput into ProcessedEvent instances.
public protocol DataTransformationService {
    func transform(modelOutput: ModelOutput, sourceId: String, sourceType: ProcessedEvent.SourceType) -> [ProcessedEvent]
}

public final class DefaultDataTransformationService: DataTransformationService {
    public init() {}

    public func transform(modelOutput: ModelOutput, sourceId: String, sourceType: ProcessedEvent.SourceType) -> [ProcessedEvent] {
        return modelOutput.items.map { item in
            ProcessedEvent(
                id: UUID().uuidString,
                source: .reminder,
                sourceType: sourceType,
                sourceId: sourceId,
                title: item.text,
                date: nil,
                start: nil,
                due: nil,
                priority: item.priority,
                tags: nil,
                dedupeKey: item.dedupeKey,
                provenance: ["llmText": .string(item.text), "explain": .string(modelOutput.explain ?? "")],
                metadata: [:],
                notes: nil,
                hash: item.text.hashValue
            )
        }
    }
}
