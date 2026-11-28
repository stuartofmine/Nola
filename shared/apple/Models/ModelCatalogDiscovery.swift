// SPDX-License-Identifier: Apache-2.0
import Foundation
import CoreFoundation

/// Directory reads never follow redirects or accumulate an unbounded response.
nonisolated final class AppleModelCatalogTransport: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    private static let budget = AppleModelCatalogBudget()
    private let lock = NSLock()
    private var continuation: CheckedContinuation<(Data, URLResponse), Error>?
    private var session: URLSession?
    private var response: URLResponse?
    private var body = Data()
    private var finished = false
    private let limit = 2 * 1024 * 1024

    static func fetch(_ request: URLRequest, configuration: URLSessionConfiguration = .ephemeral, timeout: TimeInterval = 10) async throws -> (Data, URLResponse) {
        try Task.checkCancellation()
        guard budget.acquire() else { throw URLError(.resourceUnavailable) }
        defer { budget.release() }
        configuration.urlCredentialStorage = nil
        configuration.httpCookieStorage = nil
        configuration.urlCache = nil
        configuration.timeoutIntervalForRequest = timeout
        configuration.timeoutIntervalForResource = timeout
        let transport = AppleModelCatalogTransport()
        // Consume bounded data chunks on URLSession's delegate queue. Iterating
        // millions of individual AsyncBytes could keep the model picker busy
        // long after the response arrived.
        let result = try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                transport.start(request, configuration: configuration, continuation: continuation)
            }
        } onCancel: {
            transport.finish(.failure(CancellationError()))
        }
        try Task.checkCancellation()
        return result
    }

    private func start(_ request: URLRequest, configuration: URLSessionConfiguration,
                       continuation: CheckedContinuation<(Data, URLResponse), Error>) {
        lock.lock()
        guard !finished else {
            lock.unlock()
            continuation.resume(throwing: CancellationError())
            return
        }
        self.continuation = continuation
        let session = URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
        self.session = session
        let task = session.dataTask(with: request)
        lock.unlock()
        task.resume()
    }

    private func finish(_ result: Result<(Data, URLResponse), Error>) {
        lock.lock()
        guard !finished else { lock.unlock(); return }
        finished = true
        let continuation = continuation, session = session
        self.continuation = nil; self.session = nil
        body = Data(); response = nil
        lock.unlock()
        session?.invalidateAndCancel()
        continuation?.resume(with: result)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            completionHandler(.cancel)
            finish(.failure(URLError(.badServerResponse)))
            return
        }
        guard response.expectedContentLength <= Int64(limit) else {
            completionHandler(.cancel)
            finish(.failure(URLError(.dataLengthExceedsMaximum)))
            return
        }
        lock.lock()
        let active = !finished
        if active { self.response = response }
        lock.unlock()
        completionHandler(active ? .allow : .cancel)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        lock.lock()
        guard !finished else { lock.unlock(); return }
        guard data.count <= limit - body.count else {
            lock.unlock()
            finish(.failure(URLError(.dataLengthExceedsMaximum)))
            return
        }
        body.append(data)
        lock.unlock()
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        lock.lock()
        let result: Result<(Data, URLResponse), Error>
        if let error { result = .failure(error) }
        else if let response { result = .success((body, response)) }
        else { result = .failure(URLError(.badServerResponse)) }
        lock.unlock()
        finish(result)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask,
                    willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest,
                    completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

private nonisolated final class AppleModelCatalogBudget: @unchecked Sendable {
    private let lock = NSLock()
    private var active = 0
    func acquire() -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard active < 3 else { return false }
        active += 1
        return true
    }
    func release() {
        lock.lock(); defer { lock.unlock() }
        active -= 1
    }
}


/// Discovery adapts external provider catalogs; enabled profiles remain Core-owned.
nonisolated enum AppleModelCatalogDiscovery {
    struct Model: Equatable, Sendable {
        let id: String
        let displayName: String
        var reasoningLevels: [String] = []
        var contextWindow: Int64? = nil
        var outputLimit: Int64? = nil
        var modalities: [String] = []
        var reasoningSupported: Bool? = nil
        var summarySupported: Bool? = nil
        var toolCalls: Bool? = nil
        var samplingParameters: Bool? = nil

        /// Preserve observed metadata at the Core boundary; merely listing a
        /// model never enables it or invents an inference control.
        func catalogData(wireProtocol: String) throws -> Data {
            var entry: [String: Any] = [
                "model_id": id, "display_name": displayName, "protocol": wireProtocol,
                "context_window": contextWindow.map { $0 as Any } ?? NSNull(),
                "output_limit": outputLimit.map { $0 as Any } ?? NSNull(),
                "modalities": modalities, "reasoning_levels": reasoningLevels,
                "summary_supported": summarySupported.map { $0 as Any } ?? NSNull(),
                "execution_capabilities": NSNull()
            ]
            if reasoningSupported != nil || !reasoningLevels.isEmpty || toolCalls != nil || samplingParameters != nil {
                entry["execution_capabilities"] = [
                    "toolCalls": toolCalls ?? true, "imageInput": modalities.contains("image"),
                    "reasoning": reasoningSupported ?? !reasoningLevels.isEmpty,
                    "samplingParameters": samplingParameters ?? false
                ]
            }
            return try JSONSerialization.data(withJSONObject: entry)
        }
    }

    enum DiscoveryError: LocalizedError {
        case httpStatus(Int, String?)
        case responseTooLarge(Int)
        case invalidPayload
        case localNetworkProhibited(String)

        var errorDescription: String? {
            switch self {
            case .httpStatus(let code, let detail):
                if let detail, !detail.isEmpty {
                    return "HTTP \(code): \(detail)"
                }
                return "HTTP \(code) \(HTTPURLResponse.localizedString(forStatusCode: code))"
            case .responseTooLarge(let bytes):
                return "Model catalog response exceeded size limit (\(bytes) bytes)"
            case .invalidPayload:
                return "Endpoint did not return a valid OpenAI/Anthropic model list (expected JSON with 'data' array)"
            case .localNetworkProhibited(let host):
                return "macOS blocked local network access to \(host) (Local network prohibited). Enable Nola in System Settings → Privacy & Security → Local Network, then try again."
            }
        }
    }

    static func request(endpoint: String, wireProtocol: String, secret: String) throws -> URLRequest {
        guard let base = URL(string: endpoint),
              var components = URLComponents(url: base, resolvingAgainstBaseURL: false) else {
            throw URLError(.badURL)
        }
        let path = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let basePath = path.isEmpty ? "" : "/" + path
        if wireProtocol == "anthropic", path != "v1", !path.hasSuffix("/v1") {
            components.path = basePath + "/v1/models"
        } else {
            components.path = basePath + "/models"
        }
        guard let url = components.url else { throw URLError(.badURL) }
        var request = URLRequest(url: url)
        request.timeoutInterval = 15
        if wireProtocol == "anthropic" {
            request.setValue("2023-06-01", forHTTPHeaderField: "anthropic-version")
        }
        if !secret.isEmpty {
            request.setValue(wireProtocol == "anthropic" ? secret : "Bearer " + secret,
                             forHTTPHeaderField: wireProtocol == "anthropic" ? "x-api-key" : "Authorization")
        }
        return request
    }

    static func parse(data: Data, response: URLResponse) throws -> [Model] {
        guard let httpResponse = response as? HTTPURLResponse else {
            throw URLError(.badServerResponse)
        }
        guard (200..<300).contains(httpResponse.statusCode) else {
            let detail = extractErrorMessage(from: data)
            throw DiscoveryError.httpStatus(httpResponse.statusCode, detail)
        }
        let maxCatalogBytes = 8 * 1024 * 1024
        guard data.count <= maxCatalogBytes else {
            throw DiscoveryError.responseTooLarge(data.count)
        }
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let models = object["data"] as? [[String: Any]] else {
            throw DiscoveryError.invalidPayload
        }
        return models.compactMap {
            guard let id = $0["id"] as? String, !id.isEmpty else { return nil }
            let entry = $0
            // An explicitly empty primary list stays empty. Missing metadata is
            // unknown, distinct from an advertised `none` effort.
            let rawLevels = entry["supported_reasoning_levels"] ?? entry["reasoning_levels"]
            var levels: [String] = []
            for raw in rawLevels as? [Any] ?? [] {
                if let effort = (raw as? String) ?? ((raw as? [String: Any])?["effort"] as? String),
                   !effort.isEmpty, !levels.contains(effort) { levels.append(effort) }
            }
            let limits = entry["limit"] as? [String: Any] ?? [:]
            let topProvider = entry["top_provider"] as? [String: Any] ?? [:]
            let architecture = entry["architecture"] as? [String: Any] ?? [:]
            let modes = (entry["modalities"] as? [String: Any])?["input"] as? [String]
                ?? entry["modalities"] as? [String]
                ?? architecture["input_modalities"] as? [String]
                ?? []
            let supportedParameters = entry["supported_parameters"] as? [String] ?? []
            let inferredTools: Bool? = boolean(entry["tool_call"])
                ?? (supportedParameters.isEmpty ? nil : supportedParameters.contains("tools"))
            let supportsReasoningParam = supportedParameters.contains("reasoning")
                || supportedParameters.contains("include_reasoning")
                || supportedParameters.contains("reasoning_effort")
            let inferredReasoning: Bool? = boolean(entry["reasoning"])
                ?? (supportsReasoningParam ? true : nil)
            if levels.isEmpty, supportsReasoningParam {
                levels = ["none", "low", "medium", "high"]
            }
            let inferredSampling: Bool? = boolean(entry["temperature"])
                ?? (supportedParameters.isEmpty ? nil : supportedParameters.contains("temperature"))
            return Model(id: id, displayName: entry["display_name"] as? String ?? entry["name"] as? String ?? id,
                reasoningLevels: levels,
                contextWindow: integer(
                    entry["context_window"]
                        ?? limits["context"]
                        ?? entry["context_length"]
                        ?? entry["max_model_len"]
                        ?? entry["max_context_length"]
                        ?? topProvider["context_length"]
                ),
                outputLimit: integer(
                    entry["max_output_tokens"]
                        ?? limits["output"]
                        ?? topProvider["max_completion_tokens"]
                ),
                modalities: modes, reasoningSupported: inferredReasoning,
                summarySupported: boolean(entry["supports_reasoning_summary_parameter"]),
                toolCalls: inferredTools, samplingParameters: inferredSampling)
        }
    }

    private static func extractErrorMessage(from data: Data) -> String? {
        guard !data.isEmpty, data.count <= 64 * 1024 else { return nil }
        if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            if let errObj = json["error"] as? [String: Any],
               let msg = errObj["message"] as? String, !msg.isEmpty {
                return String(msg.prefix(240))
            }
            if let msg = json["message"] as? String, !msg.isEmpty {
                return String(msg.prefix(240))
            }
            if let msg = json["error"] as? String, !msg.isEmpty {
                return String(msg.prefix(240))
            }
        }
        return nil
    }

    private static func boolean(_ value: Any?) -> Bool? {
        guard let value = value as? NSNumber, CFGetTypeID(value) == CFBooleanGetTypeID() else { return nil }
        return value.boolValue
    }

    private static func integer(_ value: Any?) -> Int64? {
        guard let value = value as? NSNumber,
              CFGetTypeID(value) != CFBooleanGetTypeID(),
              let integer = Int64(value.stringValue), integer > 0 else { return nil }
        return integer
    }

    /// Build a best-effort `/props` request for llama.cpp / Strata compatible servers.
    static func propsRequest(endpoint: String, secret: String) -> URLRequest? {
        guard let base = URL(string: endpoint),
              var components = URLComponents(url: base, resolvingAgainstBaseURL: false) else {
            return nil
        }
        var segments = components.path
            .split(separator: "/", omittingEmptySubsequences: true)
            .map(String.init)
        if segments.last == "models" { segments.removeLast() }
        if segments.last == "v1" { segments.removeLast() }
        components.path = segments.isEmpty ? "/props" : "/" + segments.joined(separator: "/") + "/props"
        components.query = nil
        guard let url = components.url else { return nil }
        var request = URLRequest(url: url)
        request.timeoutInterval = 3
        if !secret.isEmpty {
            request.setValue("Bearer " + secret, forHTTPHeaderField: "Authorization")
        }
        return request
    }

    /// Enrich discovered OpenAI-compatible models with capabilities declared by llama.cpp / Strata `/props`.
    static func enrichWithProps(models: [Model], data: Data, response: URLResponse) -> [Model] {
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return models
        }
        let templateCaps = object["chat_template_caps"] as? [String: Any] ?? [:]
        let modalitiesObj = object["modalities"] as? [String: Any] ?? [:]
        let genSettings = object["default_generation_settings"] as? [String: Any] ?? [:]
        let genParams = genSettings["params"] as? [String: Any] ?? [:]
        let chatTemplate = object["chat_template"] as? String ?? ""
        let modelAlias = (object["model_alias"] as? String)?
            .trimmingCharacters(in: .whitespacesAndNewlines)

        let supportsReasoning: Bool? = {
            if let preserve = boolean(templateCaps["supports_preserve_reasoning"]) {
                return preserve
            }
            if chatTemplate.contains("enable_thinking") || chatTemplate.contains("<think>") {
                return true
            }
            return nil
        }()
        let supportsTools: Bool? = boolean(templateCaps["supports_tool_calls"])
            ?? boolean(templateCaps["supports_tools"])
        let supportsVision = boolean(modalitiesObj["vision"]) == true
        let supportsAudio = boolean(modalitiesObj["audio"]) == true
        let propsContext = integer(genSettings["n_ctx"])
        let propsOutput = integer(genParams["n_predict"])

        return models.map { original in
            let matchesTarget = models.count == 1
                || (modelAlias != nil && !modelAlias!.isEmpty && original.id.caseInsensitiveCompare(modelAlias!) == .orderedSame)
            guard matchesTarget else { return original }
            var updated = original
            if updated.contextWindow == nil, let propsContext {
                updated.contextWindow = propsContext
            }
            if updated.outputLimit == nil, let propsOutput {
                updated.outputLimit = propsOutput
            }
            if updated.toolCalls == nil, let supportsTools {
                updated.toolCalls = supportsTools
            }
            if supportsVision {
                if updated.modalities.isEmpty { updated.modalities = ["text"] }
                if !updated.modalities.contains("image") { updated.modalities.append("image") }
            }
            if supportsAudio {
                if updated.modalities.isEmpty { updated.modalities = ["text"] }
                if !updated.modalities.contains("audio") { updated.modalities.append("audio") }
            }
            if let supportsReasoning {
                if updated.reasoningSupported == nil {
                    updated.reasoningSupported = supportsReasoning
                }
                if supportsReasoning, updated.reasoningLevels.isEmpty {
                    updated.reasoningLevels = ["none", "low", "medium", "high"]
                }
            }
            return updated
        }
    }

    static func discover(endpoint: String, wireProtocol: String, secret: String) async throws -> [Model] {
        let request = try request(endpoint: endpoint, wireProtocol: wireProtocol, secret: secret)
        do {
            let (data, response) = try await AppleModelCatalogTransport.fetch(request)
            var models = try parse(data: data, response: response)
            if wireProtocol != "anthropic",
               !models.isEmpty,
               models.contains(where: { $0.reasoningLevels.isEmpty || $0.outputLimit == nil || $0.toolCalls == nil }),
               let propsReq = propsRequest(endpoint: endpoint, secret: secret) {
                if let (propsData, propsResponse) = try? await AppleModelCatalogTransport.fetch(propsReq, timeout: 3) {
                    models = enrichWithProps(models: models, data: propsData, response: propsResponse)
                }
            }
            return models
        } catch {
            if String(describing: error).localizedCaseInsensitiveContains("local network prohibited") {
                let host = request.url?.host ?? endpoint
                throw DiscoveryError.localNetworkProhibited(host)
            }
            throw error
        }
    }
}
