// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements MLXTokenizer logic in the iOS LLM integration layer.
// - Interfaces:
//   - MLXTokenizer
// - APIs:
//   - none
//
//
//  MLXTokenizer.swift
//  Nola
//
//  Tokenizer implementation for Qwen3 models using BPE.
//

import Foundation

public class MLXTokenizer {
    private let vocab: [String: Int]
    private let reverseVocab: [Int: String]
    private let merges: [(String, String)]
    
    // Special tokens for Qwen3
    public let bosToken = "<|im_start|>"
    public let eosToken = "<|im_end|>"
    public let padToken = "<|endoftext|>"
    
    private let bosTokenId: Int
    private let eosTokenId: Int
    private let padTokenId: Int
    
    public init(vocabPath: String, mergesPath: String) throws {
        // Load vocabulary
        let vocabData = try Data(contentsOf: URL(fileURLWithPath: vocabPath))
        self.vocab = try JSONDecoder().decode([String: Int].self, from: vocabData)
        self.reverseVocab = Dictionary(uniqueKeysWithValues: vocab.map { ($1, $0) })
        
        // Load BPE merges
        let mergesContent = try String(contentsOf: URL(fileURLWithPath: mergesPath), encoding: .utf8)
        self.merges = mergesContent.split(separator: "\n")
            .dropFirst() // Skip header line
            .compactMap { line -> (String, String)? in
                let parts = line.split(separator: " ", maxSplits: 1)
                guard parts.count == 2 else { return nil }
                return (String(parts[0]), String(parts[1]))
            }
        
        // Get special token IDs
        self.bosTokenId = vocab[bosToken] ?? 0
        self.eosTokenId = vocab[eosToken] ?? vocab["<|endoftext|>"] ?? 0
        self.padTokenId = vocab[padToken] ?? vocab["<|endoftext|>"] ?? 0
    }
    
    /// Encode text to token IDs
    public func encode(_ text: String) -> [Int] {
        var tokens: [Int] = []
        
        // Simple whitespace tokenization first
        let words = text.components(separatedBy: .whitespacesAndNewlines)
        
        for word in words where !word.isEmpty {
            let wordTokens = encodeWord(word)
            tokens.append(contentsOf: wordTokens)
        }
        
        return tokens
    }
    
    private func encodeWord(_ word: String) -> [Int] {
        // Convert word to byte-level tokens
        var chars = Array(word.unicodeScalars.map { String($0) })
        
        // Apply BPE merges
        while chars.count > 1 {
            var bestPair: (Int, String, String)?
            var bestRank = Int.max
            
            // Find best merge
            for i in 0..<(chars.count - 1) {
                let pair = (chars[i], chars[i + 1])
                if let rank = merges.firstIndex(where: { $0.0 == pair.0 && $0.1 == pair.1 }) {
                    if rank < bestRank {
                        bestRank = rank
                        bestPair = (i, pair.0, pair.1)
                    }
                }
            }
            
            // Apply best merge
            guard let (idx, first, second) = bestPair else { break }
            let merged = first + second
            chars[idx] = merged
            chars.remove(at: idx + 1)
        }
        
        // Convert to token IDs
        return chars.compactMap { vocab[$0] }
    }
    
    /// Decode token IDs to text
    public func decode(_ tokens: [Int]) -> String {
        return tokens.compactMap { reverseVocab[$0] }.joined()
    }
    
    /// Add special tokens for chat format
    public func applyChatTemplate(messages: [(role: String, content: String)]) -> String {
        var prompt = ""
        
        for message in messages {
            prompt += "<|im_start|>\(message.role)\n"
            prompt += message.content
            prompt += "<|im_end|>\n"
        }
        
        // Add assistant start token
        prompt += "<|im_start|>assistant\n"
        
        return prompt
    }
}
