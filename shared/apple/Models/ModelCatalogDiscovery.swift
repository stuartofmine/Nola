// SPDX-License-Identifier: Apache-2.0
import Foundation

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

    static func fetch(_ request: URLRequest, configuration: URLSessionConfiguration = .ephemeral) async throws -> (Data, URLResponse) {
        try Task.checkCancellation()
        guard budget.acquire() else { throw URLError(.resourceUnavailable) }
        defer { budget.release() }
        configuration.urlCredentialStorage = nil
        configuration.httpCookieStorage = nil
        configuration.urlCache = nil
        configuration.timeoutIntervalForRequest = 10
        configuration.timeoutIntervalForResource = 10
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
    }

    static func request(endpoint: String, wireProtocol: String, secret: String) throws -> URLRequest {
        guard let base = URL(string: endpoint),
              var components = URLComponents(url: base, resolvingAgainstBaseURL: false) else {
            throw URLError(.badURL)
        }
        let path = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        components.path = (path.isEmpty ? "" : "/" + path) + "/models"
        guard let url = components.url else { throw URLError(.badURL) }
        var request = URLRequest(url: url)
        request.timeoutInterval = 10
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
        guard let response = response as? HTTPURLResponse, (200..<300).contains(response.statusCode),
              data.count <= 2 * 1024 * 1024,
              let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let models = object["data"] as? [[String: Any]] else {
            throw URLError(.badServerResponse)
        }
        return models.compactMap {
            guard let id = $0["id"] as? String, !id.isEmpty else { return nil }
            return Model(id: id, displayName: ($0["display_name"] as? String) ?? id)
        }
    }

    static func discover(endpoint: String, wireProtocol: String, secret: String) async throws -> [Model] {
        let request = try request(endpoint: endpoint, wireProtocol: wireProtocol, secret: secret)
        let (data, response) = try await AppleModelCatalogTransport.fetch(request)
        return try parse(data: data, response: response)
    }
}
