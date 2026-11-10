// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements MLXModel logic in the iOS LLM integration layer.
// - Interfaces:
//   - MLXModelConfig
//   - MLXModelWeights
//   - Qwen3Model
//   - KVCache
//   - SamplingConfig
//   - Sampler
// - APIs:
//   - none
//
//
//  MLXModel.swift
//  Nola
//
//  Qwen3 model implementation for MLX Swift.
//  Requires mlx-swift package added via Xcode SPM.
//

import Foundation
import MLX
import MLXNN
import MLXRandom
import MLXFast

// MARK: - Model Configuration

public struct MLXModelConfig: Codable {
    let architectures: [String]?
    let hiddenSize: Int
    let intermediateSize: Int
    let numAttentionHeads: Int
    let numKeyValueHeads: Int?
    let numHiddenLayers: Int
    let vocabSize: Int
    let maxPositionEmbeddings: Int?
    let rmsNormEps: Double?
    let ropeTheta: Double?
    
    enum CodingKeys: String, CodingKey {
        case architectures
        case hiddenSize = "hidden_size"
        case intermediateSize = "intermediate_size"
        case numAttentionHeads = "num_attention_heads"
        case numKeyValueHeads = "num_key_value_heads"
        case numHiddenLayers = "num_hidden_layers"
        case vocabSize = "vocab_size"
        case maxPositionEmbeddings = "max_position_embeddings"
        case rmsNormEps = "rms_norm_eps"
        case ropeTheta = "rope_theta"
    }
}

// MARK: - Model Weights Loader

public class MLXModelWeights {
    private let modelPath: URL
    
    public init(modelPath: URL) {
        self.modelPath = modelPath
    }
    
    /// Load model weights from safetensors file
    public func loadWeights() throws -> [String: MLXArray] {
        let weightsPath = modelPath.appendingPathComponent("model.safetensors")
        
        guard FileManager.default.fileExists(atPath: weightsPath.path) else {
            throw NolaError.modelLoadFailed(reason: "model.safetensors not found")
        }
        
        Logger.shared.log("Loading weights from: \(weightsPath.path)", level: .info)
        
        // Load safetensors using MLX
        let arrays = try MLX.loadArrays(url: weightsPath)
        Logger.shared.log("Loaded \(arrays.count) weight tensors", level: .info)
        
        return arrays
    }
}

// MARK: - Qwen3 Model

public class Qwen3Model {
    private let config: MLXModelConfig
    private var weights: [String: MLXArray]
    
    // Model components
    private var embeddings: MLXArray?
    private var outputProjection: MLXArray?
    private var normWeight: MLXArray?
    
    public init(config: MLXModelConfig, weights: [String: MLXArray]) {
        self.config = config
        self.weights = weights
        self.setupLayers()
    }
    
    private func setupLayers() {
        // For 4-bit quantized models, dequantize embedding layer if needed
        if let embWeight = weights["model.embed_tokens.weight"],
           let embScales = weights["model.embed_tokens.scales"],
           let embBiases = weights["model.embed_tokens.biases"] {
            // Calculate group size from shapes:
            // weight: [vocab, hidden/8], scales: [vocab, num_groups]
            // num_groups = (hidden/8) / group_size * 8/bits = hidden / group_size
            // So: group_size = hidden / num_groups
            let numGroups = embScales.shape[1]
            let embGroupSize = config.hiddenSize / numGroups
            Logger.shared.log("Embedding quantization: weight=\(embWeight.shape), scales=\(embScales.shape), groupSize=\(embGroupSize)", level: .debug)
            
            // Dequantize embeddings: [vocab_size, hidden_size/8] uint32 -> [vocab_size, hidden_size] float
            embeddings = MLX.dequantized(embWeight, scales: embScales, biases: embBiases, groupSize: embGroupSize, bits: 4)
            Logger.shared.log("Dequantized embeddings from uint32 to float", level: .debug)
        } else {
            // Not quantized, use as-is
            embeddings = weights["model.embed_tokens.weight"]
        }
        
        outputProjection = weights["lm_head.weight"]
        normWeight = weights["model.norm.weight"]

        // Log critical weight shapes for diagnostics
        if let emb = embeddings {
            let s = "[" + emb.shape.map(String.init).joined(separator: ",") + "]"
            Logger.shared.log("embeddings shape=\(s), dtype=\(emb.dtype)", level: .debug)
        } else {
            Logger.shared.log("embeddings weight missing", level: .error)
        }

        if let lm = outputProjection {
            let s = "[" + lm.shape.map(String.init).joined(separator: ",") + "]"
            Logger.shared.log("lm_head.weight shape=\(s), dtype=\(lm.dtype)", level: .debug)
        } else {
            Logger.shared.log("lm_head.weight missing", level: .error)
        }

        if let nw = normWeight {
            let s = "[" + nw.shape.map(String.init).joined(separator: ",") + "]"
            Logger.shared.log("model.norm.weight shape=\(s)", level: .debug)
        } else {
            Logger.shared.log("model.norm.weight missing", level: .error)
        }
    }
    
    /// Helper function to perform matmul with potential quantized weights
    private func matmulMaybeQuantized(_ x: MLXArray, weight: MLXArray, prefix: String) -> MLXArray {
        // Check if quantized (has scales)
        if let scales = weights[prefix + ".scales"],
           let biases = weights[prefix + ".biases"] {
            // Calculate group size from weight and scales shapes
            // weight: [out, in/8], scales: [out, num_groups]
            // For transpose=true: result = x @ weight.T
            // num_groups = (in/8) / group_size * 8/bits = in / group_size
            // So: group_size = in / num_groups
            let numGroups = scales.shape[1]
            let inputDim = weight.shape[1] * 8 // weight is packed: [out, in/8]
            let groupSize = inputDim / numGroups
            
            let bits = 4
            return MLX.quantizedMM(x, weight, scales: scales, biases: biases, transpose: true, groupSize: groupSize, bits: bits)
        } else {
            // Regular matmul for unquantized weights
            return MLX.matmul(x, weight)
        }
    }

    
    /// Forward pass through the model
    /// - Parameters:
    ///   - inputIds: Token IDs [sequence_length]
    ///   - cache: Optional KV cache for faster inference
    /// - Returns: Logits [sequence_length, vocab_size]
    public func forward(inputIds: [Int], cache: KVCache? = nil) async throws -> [[Float]] {
        Logger.shared.log("Forward pass with \(inputIds.count) tokens", level: .debug)
        
        guard let embeddings = embeddings else {
            throw NolaError.modelLoadFailed(reason: "Embeddings not loaded")
        }
        
        // 1. Embedding lookup (now always full precision after setupLayers dequantization)
        let inputArray = MLXArray(inputIds.map { Int32($0) })
        let embShape = embeddings.shape
        let embShapeStr = "[" + embShape.map(String.init).joined(separator: ",") + "]"
        Logger.shared.log("Embeddings shape=\(embShapeStr), vocabSize=\(config.vocabSize), hiddenSize=\(config.hiddenSize)", level: .debug)

        var hidden: MLXArray
        if embShape.count == 2, embShape[0] == config.vocabSize {
            hidden = MLX.take(embeddings, inputArray, axis: 0)
        } else if embShape.count == 2, embShape[1] == config.vocabSize {
            hidden = MLX.take(embeddings, inputArray, axis: 1).T
        } else {
            Logger.shared.log("Unexpected embedding shape, defaulting axis=0 gather", level: .warn)
            hidden = MLX.take(embeddings, inputArray, axis: 0)
        }

        let hidShapeStr = "[" + hidden.shape.map(String.init).joined(separator: ",") + "]"
        Logger.shared.log("Hidden(after embedding) shape=\(hidShapeStr)", level: .debug)
        
        // 2. Process through transformer layers
        for layerIdx in 0..<config.numHiddenLayers {
            hidden = try processLayer(hidden, layerIdx: layerIdx, cache: cache)
        }
        
        // 3. Final normalization
        if let normWeight = normWeight {
            hidden = rmsNorm(hidden, weight: normWeight, label: "final_norm")
        }
        
        // 4. Output projection (handle quantized lm_head)
        guard let outputProjection = outputProjection else {
            throw NolaError.modelLoadFailed(reason: "Output projection not loaded")
        }
        
        let logits = matmulMaybeQuantized(hidden, weight: outputProjection, prefix: "lm_head")
        
        // Convert to Swift array
        return convertToSwiftArray(logits)
    }
    
    private func processLayer(_ hidden: MLXArray, layerIdx: Int, cache: KVCache?) throws -> MLXArray {
        let prefix = "model.layers.\(layerIdx)"
        
        // Self-attention with residual
        var residual = hidden
        var x = rmsNorm(hidden, weight: weights["\(prefix).input_layernorm.weight"], label: "layer_\(layerIdx)_input_layernorm")
        x = try attention(x, layerIdx: layerIdx, cache: cache)
        x = residual + x
        
        // FFN with residual
        residual = x
        x = rmsNorm(x, weight: weights["\(prefix).post_attention_layernorm.weight"], label: "layer_\(layerIdx)_post_attention_layernorm")
        x = try feedForward(x, layerIdx: layerIdx)
        x = residual + x
        
        return x
    }
    
    private func rmsNorm(_ x: MLXArray, weight: MLXArray?, label: String) -> MLXArray {
        guard let weight = weight else { return x }
        let xShape = "[" + x.shape.map(String.init).joined(separator: ",") + "]"
        let wShape = "[" + weight.shape.map(String.init).joined(separator: ",") + "]"
        Logger.shared.log("rmsNorm(\(label)) x.shape=\(xShape) weight.shape=\(wShape)", level: .debug)
        return MLX.rmsNorm(x, weight: weight, eps: Float(config.rmsNormEps ?? 1e-6))
    }
    
    private func attention(_ x: MLXArray, layerIdx: Int, cache: KVCache?) throws -> MLXArray {
        let prefix = "model.layers.\(layerIdx).self_attn"
        
        guard let qWeight = weights["\(prefix).q_proj.weight"],
              let kWeight = weights["\(prefix).k_proj.weight"],
              let vWeight = weights["\(prefix).v_proj.weight"],
              let oWeight = weights["\(prefix).o_proj.weight"] else {
            throw NolaError.modelLoadFailed(reason: "Attention weights missing for layer \(layerIdx)")
        }
        
        // QKV projections (handle quantized weights)
        let q = matmulMaybeQuantized(x, weight: qWeight, prefix: "\(prefix).q_proj")
        var k = matmulMaybeQuantized(x, weight: kWeight, prefix: "\(prefix).k_proj")
        var v = matmulMaybeQuantized(x, weight: vWeight, prefix: "\(prefix).v_proj")
        
        // Apply RoPE (Rotary Position Embeddings)
        let headDim = config.hiddenSize / config.numAttentionHeads
        
        // Update cache if provided
        if let cache = cache {
            k = cache.updateKeys(k, layerIdx: layerIdx)
            v = cache.updateValues(v, layerIdx: layerIdx)
        }
        
        // Scaled dot-product attention
        let scores = MLX.matmul(q, k.T) / sqrt(Float(headDim))
        let attnWeights = MLX.softmax(scores, axis: -1)
        let attnOutput = MLX.matmul(attnWeights, v)
        
        // Output projection
        return matmulMaybeQuantized(attnOutput, weight: oWeight, prefix: "\(prefix).o_proj")
    }
    
    private func feedForward(_ x: MLXArray, layerIdx: Int) throws -> MLXArray {
        let prefix = "model.layers.\(layerIdx).mlp"
        
        guard let gateWeight = weights["\(prefix).gate_proj.weight"],
              let upWeight = weights["\(prefix).up_proj.weight"],
              let downWeight = weights["\(prefix).down_proj.weight"] else {
            throw NolaError.modelLoadFailed(reason: "FFN weights missing for layer \(layerIdx)")
        }
        
        // SwiGLU: gate(x) * up(x) (handle quantized weights)
        let gate = matmulMaybeQuantized(x, weight: gateWeight, prefix: "\(prefix).gate_proj")
        let up = matmulMaybeQuantized(x, weight: upWeight, prefix: "\(prefix).up_proj")
        // SiLU(x) = x * sigmoid(x)
        let activated = gate * MLX.sigmoid(gate) * up
        
        // Down projection
        return matmulMaybeQuantized(activated, weight: downWeight, prefix: "\(prefix).down_proj")
    }
    
    private func convertToSwiftArray(_ mlxArray: MLXArray) -> [[Float]] {
        // Convert MLXArray to Swift 2D array
        let shape = mlxArray.shape
        guard shape.count == 2 else { return [] }
        
        let seqLen = shape[0]
        let vocabSize = shape[1]
        
        let flatArray = mlxArray.asArray(Float.self)
        var result: [[Float]] = []
        
        for i in 0..<seqLen {
            let start = i * vocabSize
            let end = start + vocabSize
            result.append(Array(flatArray[start..<end]))
        }
        
        return result
    }
}

// MARK: - KV Cache for efficient generation

public class KVCache {
    private var keysCache: [Int: MLXArray] = [:]
    private var valuesCache: [Int: MLXArray] = [:]
    
    public init() {}
    
    public func updateKeys(_ newKeys: MLXArray, layerIdx: Int) -> MLXArray {
        if let existingKeys = keysCache[layerIdx] {
            let concatenated = MLX.concatenated([existingKeys, newKeys], axis: 0)
            keysCache[layerIdx] = concatenated
            return concatenated
        } else {
            keysCache[layerIdx] = newKeys
            return newKeys
        }
    }
    
    public func updateValues(_ newValues: MLXArray, layerIdx: Int) -> MLXArray {
        if let existingValues = valuesCache[layerIdx] {
            let concatenated = MLX.concatenated([existingValues, newValues], axis: 0)
            valuesCache[layerIdx] = concatenated
            return concatenated
        } else {
            valuesCache[layerIdx] = newValues
            return newValues
        }
    }
    
    public func reset() {
        keysCache.removeAll()
        valuesCache.removeAll()
    }
}

// MARK: - Sampling utilities

public struct SamplingConfig {
    let temperature: Float
    let topP: Float
    let topK: Int?
    
    public init(temperature: Float = 0.7, topP: Float = 0.9, topK: Int? = nil) {
        self.temperature = temperature
        self.topP = topP
        self.topK = topK
    }
}

public class Sampler {
    /// Sample next token from logits
    public static func sample(logits: [Float], config: SamplingConfig) -> Int {
        guard !logits.isEmpty else { return 0 }
        
        // Apply temperature
        var scaledLogits = logits.map { $0 / config.temperature }
        
        // Softmax
        let maxLogit = scaledLogits.max() ?? 0
        let expLogits = scaledLogits.map { exp($0 - maxLogit) }
        let sumExp = expLogits.reduce(0, +)
        let probs = expLogits.map { $0 / sumExp }
        
        // Top-k filtering
        if let topK = config.topK, topK < probs.count {
            var indexed = probs.enumerated().map { ($0, $1) }
            indexed.sort { $0.1 > $1.1 }
            let topKIndices = Set(indexed.prefix(topK).map { $0.0 })
            
            scaledLogits = scaledLogits.enumerated().map { idx, val in
                topKIndices.contains(idx) ? val : -Float.infinity
            }
        }
        
        // Top-p (nucleus) sampling
        var indexed = probs.enumerated().map { ($0, $1) }
        indexed.sort { $0.1 > $1.1 }
        
        var cumProb: Float = 0
        var nucleusIndices = Set<Int>()
        
        for (idx, prob) in indexed {
            cumProb += prob
            nucleusIndices.insert(idx)
            if cumProb >= config.topP {
                break
            }
        }
        
        // Sample from nucleus
        let nucleusProbs = probs.enumerated().map { idx, prob in
            nucleusIndices.contains(idx) ? prob : 0
        }
        let nucleusSum = nucleusProbs.reduce(0, +)
        
        guard nucleusSum > 0 else { return 0 }
        
        let normalizedProbs = nucleusProbs.map { $0 / nucleusSum }
        
        // Categorical sampling
        let rand = Float.random(in: 0...1)
        var cumSum: Float = 0
        
        for (idx, prob) in normalizedProbs.enumerated() {
            cumSum += prob
            if cumSum >= rand {
                return idx
            }
        }
        
        return 0
    }
}
