// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements OpenAIResponseFixupURLProtocol logic in the iOS LLM integration layer.
// - Interfaces:
//   - OpenAIResponseFixupURLProtocol
// - APIs:
//   - none
//
import Foundation

/// URLProtocol hook that normalizes OpenAI-compatible JSON responses for the MacPaw/OpenAI SDK.
///
/// Some OpenAI-compatible gateways/providers omit top-level fields like `created`.
/// The SDK expects them to exist, so we inject missing fields on the client side
/// while still using the SDK for networking and decoding.
final class OpenAIResponseFixupURLProtocol: URLProtocol {
    private static let handledKey = "OpenAIResponseFixupURLProtocol.handled"
    private static var isRegistered = false
    private static let lock = NSLock()

    // Hosts that should have responses normalized.
    private static var enabledHosts: Set<String> = []

    static func enable(for baseURL: URL) {
        guard let host = baseURL.host, !host.isEmpty else { return }

        lock.lock()
        enabledHosts.insert(host)
        let shouldRegister = !isRegistered
        if shouldRegister { isRegistered = true }
        lock.unlock()

        if shouldRegister {
            URLProtocol.registerClass(OpenAIResponseFixupURLProtocol.self)
        }
    }

    override class func canInit(with request: URLRequest) -> Bool {
        guard URLProtocol.property(forKey: handledKey, in: request) == nil else { return false }
        guard let url = request.url, let host = url.host else { return false }

        lock.lock()
        let enabled = enabledHosts.contains(host)
        lock.unlock()
        guard enabled else { return false }

        // Only touch OpenAI API paths.
        // Be tolerant to gateways that don't include `/v1` in the visible path.
        let path = url.path.lowercased()
        if path.contains("chat/completions") || path.hasSuffix("/completions") {
            return true
        }
        return false
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        guard let url = request.url else {
            client?.urlProtocol(self, didFailWithError: URLError(.badURL))
            return
        }

        // Mark the request as handled to avoid infinite recursion.
        let mutable = (request as NSURLRequest).mutableCopy() as? NSMutableURLRequest
        guard let mutableRequest = mutable else {
            client?.urlProtocol(self, didFailWithError: URLError(.cannotCreateFile))
            return
        }
        URLProtocol.setProperty(true, forKey: Self.handledKey, in: mutableRequest)
        let newRequest = mutableRequest as URLRequest

        // Use a URLSession that does NOT include this protocol to avoid recursion.
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = []
        let session = URLSession(configuration: config)

        let debugEnabled = ProcessInfo.processInfo.environment["NOLA_DEBUG_OPENAI_URLPROTOCOL"] == "1"
        if debugEnabled {
            print("[OpenAIResponseFixupURLProtocol] intercept -> \(url.absoluteString)")
        }

        let task = session.dataTask(with: newRequest) { [weak self] data, response, error in
            guard let self else { return }

            if let error {
                self.client?.urlProtocol(self, didFailWithError: error)
                return
            }

            guard let response else {
                self.client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
                return
            }

            // Default passthrough if we can't/shouldn't rewrite.
            func passthrough(_ body: Data) {
                self.client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
                self.client?.urlProtocol(self, didLoad: body)
                self.client?.urlProtocolDidFinishLoading(self)
            }

            guard let http = response as? HTTPURLResponse,
                  let data,
                  (200...299).contains(http.statusCode)
            else {
                passthrough(data ?? Data())
                return
            }

            // Attempt to rewrite JSON. Prefer checking Content-Type, but be tolerant:
            // some gateways mislabel JSON responses (or omit Content-Type).
            let contentType = (http.value(forHTTPHeaderField: "Content-Type") ?? "").lowercased()
            let looksLikeJSON: Bool = {
                if contentType.contains("application/json") { return true }
                if let first = data.first, first == UInt8(ascii: "{") || first == UInt8(ascii: "[") { return true }
                return false
            }()
            guard looksLikeJSON else {
                passthrough(data)
                return
            }

            do {
                let obj = try JSONSerialization.jsonObject(with: data, options: [])
                guard var dict = obj as? [String: Any] else {
                    passthrough(data)
                    return
                }

                if dict["created"] == nil {
                    dict["created"] = Int(Date().timeIntervalSince1970)
                    if debugEnabled {
                        print("[OpenAIResponseFixupURLProtocol] injected created")
                    }
                }

                let newData = try JSONSerialization.data(withJSONObject: dict, options: [])

                var headers = http.allHeaderFields
                headers["Content-Length"] = String(newData.count)
                headers["Content-Type"] = "application/json"

                let newResponse = HTTPURLResponse(
                    url: url,
                    statusCode: http.statusCode,
                    httpVersion: "HTTP/1.1",
                    headerFields: headers as? [String: String]
                ) ?? http

                self.client?.urlProtocol(self, didReceive: newResponse, cacheStoragePolicy: .notAllowed)
                self.client?.urlProtocol(self, didLoad: newData)
                self.client?.urlProtocolDidFinishLoading(self)
            } catch {
                // If parsing fails, pass through original bytes.
                passthrough(data)
            }
        }

        task.resume()
    }

    override func stopLoading() {
        // URLSession tasks are short-lived here; nothing to cancel.
    }
}
