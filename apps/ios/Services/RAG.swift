// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements RAG logic in the iOS service layer.
// - Interfaces:
//   - RagVectorStore
//   - RagChunk
//   - RagChunkMatch
//   - RagEmbedder
//   - PlaceholderVectorStore
//   - PlaceholderEmbedder
//   - RagPipeline
//   - RagStatus
// - APIs:
//   - none
//
//
//  RAG.swift
//  Nola
//
//  RAG (Retrieval-Augmented Generation) protocol definitions and placeholders.
//  Actual implementation will be added in v0.4+.
//

import Foundation

// MARK: - Vector Store Protocol

/// Protocol for storing and querying vector embeddings.
public protocol RagVectorStore {
    func upsert(chunks: [RagChunk]) async throws
    func query(vector: [Float], topK: Int) async throws -> [RagChunkMatch]
    func deleteAll() async throws
    func count() async throws -> Int
}

/// A chunk of text with its embedding.
public struct RagChunk: Identifiable, Hashable {
    public let id: String
    public let sourceType: String // "calendar", "reminder", "imported_doc"
    public let sourceId: String
    public let text: String
    public let embedding: [Float]
    public let createdAt: Date
    
    public init(id: String, sourceType: String, sourceId: String, text: String, embedding: [Float], createdAt: Date) {
        self.id = id
        self.sourceType = sourceType
        self.sourceId = sourceId
        self.text = text
        self.embedding = embedding
        self.createdAt = createdAt
    }
}

/// Result of a vector search.
public struct RagChunkMatch {
    public let chunk: RagChunk
    public let score: Float // Cosine similarity or distance
    
    public init(chunk: RagChunk, score: Float) {
        self.chunk = chunk
        self.score = score
    }
}

// MARK: - Embedder Protocol

/// Protocol for generating embeddings from text.
public protocol RagEmbedder {
    func embed(texts: [String]) async throws -> [[Float]]
}

// MARK: - Placeholder Implementations

/// Placeholder vector store that returns empty results.
public final class PlaceholderVectorStore: RagVectorStore {
    public init() {}
    
    public func upsert(chunks: [RagChunk]) async throws {
        // No-op
    }
    
    public func query(vector: [Float], topK: Int) async throws -> [RagChunkMatch] {
        return []
    }
    
    public func deleteAll() async throws {
        // No-op
    }
    
    public func count() async throws -> Int {
        return 0
    }
}

/// Placeholder embedder that returns zero vectors.
public final class PlaceholderEmbedder: RagEmbedder {
    private let dimension: Int
    
    public init(dimension: Int = 384) {
        self.dimension = dimension
    }
    
    public func embed(texts: [String]) async throws -> [[Float]] {
        return texts.map { _ in Array(repeating: 0.0, count: dimension) }
    }
}

// MARK: - RAG Pipeline (stub)

/// High-level RAG pipeline coordinator (to be implemented).
public final class RagPipeline {
    private let vectorStore: RagVectorStore
    private let embedder: RagEmbedder
    public var enabled: Bool = false
    
    public init(vectorStore: RagVectorStore = PlaceholderVectorStore(), embedder: RagEmbedder = PlaceholderEmbedder()) {
        self.vectorStore = vectorStore
        self.embedder = embedder
    }
    
    /// Retrieve context for a query.
    public func retrieveContext(query: String, topK: Int = 5) async throws -> [RagChunkMatch] {
        guard enabled else { return [] }
        
        let vectors = try await embedder.embed(texts: [query])
        guard let queryVector = vectors.first else { return [] }
        
        return try await vectorStore.query(vector: queryVector, topK: topK)
    }
    
    /// Index new documents.
    public func indexDocuments(_ docs: [(id: String, text: String, sourceType: String)]) async throws {
        guard enabled else { return }
        
        let texts = docs.map { $0.text }
        let embeddings = try await embedder.embed(texts: texts)
        
        let chunks = zip(docs, embeddings).map { doc, embedding in
            RagChunk(
                id: doc.id,
                sourceType: doc.sourceType,
                sourceId: doc.id,
                text: doc.text,
                embedding: embedding,
                createdAt: Date()
            )
        }
        
        try await vectorStore.upsert(chunks: chunks)
    }
    
    /// Get index status.
    public func getStatus() async throws -> RagStatus {
        let count = try await vectorStore.count()
        return RagStatus(enabled: enabled, documentCount: count, lastUpdated: Date())
    }
}

public struct RagStatus {
    public let enabled: Bool
    public let documentCount: Int
    public let lastUpdated: Date
}
