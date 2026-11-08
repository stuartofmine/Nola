// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelRegistry logic in the iOS LLM integration layer.
// - Interfaces:
//   - AggregatedModel
//   - ModelRegistry
// - APIs:
//   - none
//
import Foundation

public struct AggregatedModel: Identifiable, Hashable {
    public enum Source: String, Codable {
        case local
        case cloud
    }

    public let id: String
    public let name: String
    public let source: Source
    public let description: String?
    public let available: Bool

    public init(id: String, name: String, source: Source, description: String? = nil, available: Bool = false) {
        self.id = id
        self.name = name
        self.source = source
        self.description = description
        self.available = available
    }
}

/// Aggregates models from local downloaded models and cloud provider models.
public enum ModelRegistry {
    public static func fetchAggregated(container: ServiceContainer, httpClient: HTTPClient? = nil) async -> [AggregatedModel] {
        let httpClient = httpClient ?? DefaultHTTPClient()
        var out: [AggregatedModel] = []

        // 1) Local downloaded models
        let localSet = container.modelDownloader.downloadedModels
        for modelType in ModelDownloader.ModelType.allCases {
            let id = modelType.rawValue
            let name = modelType.displayName
            let available = localSet.contains(modelType)
            let description = modelType.description
            out.append(AggregatedModel(id: id, name: name, source: .local, description: description, available: available))
        }

        // 2) Cloud models (try to call baseURL/v1/models)
        if let base = URL(string: container.baseURLString), !container.apiKey.isEmpty {
            let endpoint = base.appendingPathComponent("v1/models")
            var request = URLRequest(url: endpoint)
            request.httpMethod = "GET"
            request.setValue("Bearer \(container.apiKey)", forHTTPHeaderField: "Authorization")
            do {
                let (data, http) = try await httpClient.perform(request)
                if (200...299).contains(http.statusCode) {
                    if let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                        // OpenAI style: { "data": [{"id":"gpt-4"}, ...] }
                        if let arr = root["data"] as? [[String: Any]] {
                            for obj in arr {
                                if let id = obj["id"] as? String {
                                    let name = id
                                    out.append(AggregatedModel(id: id, name: name, source: .cloud, description: nil, available: true))
                                }
                            }
                        }
                    }
                }
            } catch {
                // ignore network errors; cloud list is optional
                container.logger.log(.debug, "ModelRegistry", "Failed to fetch cloud models: \(error.localizedDescription)")
            }
        }

        return out
    }
}
