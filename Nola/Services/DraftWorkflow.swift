// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements DraftWorkflow logic in the iOS service layer.
// - Interfaces:
//   - LLMDraft
//   - DraftCandidate
//   - DraftWorkflowManager
//   - DraftStorage
// - APIs:
//   - none
//
//
//  DraftWorkflow.swift
//  Nola
//
//  Draft-Confirm-Persist workflow for LLM-generated suggestions

import Foundation

// MARK: - Draft Types

public struct LLMDraft: Identifiable, Codable {
    public let id: String
    public var title: String
    public var description: String?
    public var suggestedAction: String
    public var confidence: Double
    public var source: String // "calendar", "health", "reminder", "task"
    public var metadata: [String: String]?
    
    public let createdAt: Date
    public var confirmedAt: Date?
    public var rejected: Bool = false
    
    enum CodingKeys: String, CodingKey {
        case id, title, description, suggestedAction, confidence, source, metadata
        case createdAt, confirmedAt, rejected
    }
}

public struct DraftCandidate: Identifiable, Codable {
    public let id: String
    public var drafts: [LLMDraft]
    public var status: Status
    public let createdAt: Date
    
    public enum Status: String, Codable {
        case pending = "pending"
        case reviewed = "reviewed"
        case confirmed = "confirmed"
        case rejected = "rejected"
    }
}

// MARK: - Draft Workflow Manager

public final class DraftWorkflowManager {
    public init() { }
    
    // MARK: - Create Drafts
    
    /// Create drafts from LLM output using array of items with standard properties
    public func createDrafts(from items: [[String: Any]], source: String) -> [LLMDraft] {
        return items.enumerated().map { index, item in
            let text = (item["text"] as? String) ?? "Unknown"
            let priority = (item["priority"] as? Double) ?? 0.5
            
            return LLMDraft(
                id: UUID().uuidString,
                title: text.prefix(100).trimmingCharacters(in: .whitespaces),
                description: text.count > 100 ? String(text.dropFirst(100)) : nil,
                suggestedAction: "create_event",
                confidence: priority,
                source: source,
                metadata: nil,
                createdAt: Date(),
                confirmedAt: nil,
                rejected: false
            )
        }
    }
    
    /// Create a draft candidate batch
    public func createCandidate(drafts: [LLMDraft]) -> DraftCandidate {
        DraftCandidate(
            id: UUID().uuidString,
            drafts: drafts,
            status: .pending,
            createdAt: Date()
        )
    }
    
    // MARK: - Review & Edit Drafts
    
    /// Update a draft with user edits
    public func editDraft(_ draft: inout LLMDraft, title: String?, description: String?, action: String?) {
        if let title = title {
            draft.title = title
        }
        if let description = description {
            draft.description = description
        }
        if let action = action {
            draft.suggestedAction = action
        }
    }
    
    /// Reject a draft
    public func rejectDraft(_ draft: inout LLMDraft) {
        draft.rejected = true
    }
    
    // MARK: - Confirm Drafts
    
    /// Confirm selected drafts (returns dictionary representation)
    public func confirmDrafts(_ drafts: [LLMDraft]) -> [[String: Any]] {
        return drafts.compactMap { draft in
            guard !draft.rejected else { return nil }
            
            return [
                "id": UUID().uuidString,
                "requestId": "draft-\(draft.id)",
                "title": draft.title,
                "description": draft.description ?? "",
                "priority": draft.confidence,
                "source": draft.source,
                "dedupeKey": draft.id,
                "confirmedAt": ISO8601DateFormatter().string(from: Date())
            ]
        }
    }
}

// MARK: - Draft Storage Helper

public final class DraftStorage {
    private var pendingDrafts: [String: [LLMDraft]] = [:]
    
    public init() { }
    
    /// Save drafts temporarily
    public func savePendingDrafts(_ drafts: [LLMDraft], for candidateId: String) {
        pendingDrafts[candidateId] = drafts
    }
    
    /// Retrieve pending drafts
    public func getPendingDrafts(for candidateId: String) -> [LLMDraft]? {
        return pendingDrafts[candidateId]
    }
    
    /// Clear confirmed drafts
    public func clearPendingDrafts(for candidateId: String) {
        pendingDrafts.removeValue(forKey: candidateId)
    }
    
    /// Get all pending candidate IDs
    public func getPendingCandidateIds() -> [String] {
        Array(pendingDrafts.keys)
    }
}
