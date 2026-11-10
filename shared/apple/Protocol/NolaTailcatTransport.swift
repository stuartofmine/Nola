// SPDX-License-Identifier: Apache-2.0
// Optional ephemeral network transport. Logical Host identity and credential
// storage are never replaced by a temporary loopback port.
import Foundation
import CryptoKit
#if canImport(NolaTailcat)
import NolaTailcat
#endif

nonisolated enum NolaTailcatTransport {
    struct Configuration: Sendable, Equatable {
        let origin: URL
        let token: String
        let hostID: String
        let generation: UUID
    }
    private static let lock = NSLock()
    nonisolated(unsafe) private static var configurations: [String: Configuration] = [:]
    private static let lifecycle: Void = {
        NotificationCenter.default.addObserver(forName: .init("UIApplicationDidEnterBackgroundNotification"), object: nil, queue: nil) { _ in
            Task { await NolaTailcatTransport.park() }
        }
        NotificationCenter.default.addObserver(forName: .init("UIApplicationWillEnterForegroundNotification"), object: nil, queue: nil) { _ in
            Task { await NolaTailcatTransport.resume() }
        }
        NotificationCenter.default.addObserver(forName: .init("NSApplicationWillTerminateNotification"), object: nil, queue: nil) { _ in
            Task { await NolaTailcatTransport.suspend() }
        }
    }()
    static func park() async { await NolaTailcatConnections.shared.park() }
    static func resume() async { await NolaTailcatConnections.shared.resume() }
    static func suspend() async { await NolaTailcatConnections.shared.suspend() }
    static var available: Bool {
        #if canImport(NolaTailcat)
        true
        #else
        false
        #endif
    }
    static func origin(_ url: URL) -> URL? {
        guard var parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              parts.user == nil, parts.password == nil, parts.scheme == "http", parts.host != nil else { return nil }
        parts.path = ""; parts.query = nil; parts.fragment = nil
        return parts.url
    }
    static func register(baseURL: URL, token: String, hostID: String?, certificatePin: String? = nil) {
        guard available, MobileHostTransportPolicy.allows(baseURL), certificatePin?.isEmpty != false, ["", "/", "/api", "/api/mobile", "/api/mobile/"].contains(baseURL.path), let origin = origin(baseURL),
              let hostID, !hostID.isEmpty, !token.isEmpty else { return }
        _ = lifecycle
        lock.withLock {
            if let current = configurations[origin.absoluteString], current.token == token, current.hostID == hostID { return }
            configurations[origin.absoluteString] = Configuration(origin: origin, token: token, hostID: hostID, generation: UUID())
        }
    }
    static func unregister(baseURL: URL) {
        guard let key = origin(baseURL)?.absoluteString else { return }
        let previous = lock.withLock { configurations.removeValue(forKey: key) }
        if let previous { Task { await NolaTailcatConnections.shared.remove(key, generation: previous.generation) } }
    }
    static func configuration(for url: URL) -> Configuration? {
        guard let key = origin(url)?.absoluteString else { return nil }
        lock.lock(); defer { lock.unlock() }
        return configurations[key]
    }
    #if os(macOS)
    // A desktop browser can use the App-owned loopback bridge. Do not use this
    // for mobile external browsers: their owner App may be OS-suspended.
    static func browserURL(for logicalURL: URL) async throws -> URL {
        guard let configuration = configuration(for: logicalURL) else { return logicalURL }
        let endpoint = try await NolaTailcatConnections.shared.endpoint(for: configuration)
        guard var parts = URLComponents(url: logicalURL, resolvingAgainstBaseURL: false) else { throw URLError(.badURL) }
        parts.scheme = endpoint.scheme; parts.host = endpoint.host; parts.port = endpoint.port
        guard let url = parts.url else { throw URLError(.badURL) }; return url
    }
    #endif
    // Select a route using the request's frozen credential. This is not the
    // permission authority: the Host still validates proof, body and ownership.
    static func matches(_ request: URLRequest, configuration: Configuration) -> Bool {
        guard let url = request.url, url.path.hasPrefix("/api/mobile/"),
              let timestamp = request.value(forHTTPHeaderField: "X-Nola-Mobile-Timestamp"),
              let nonce = request.value(forHTTPHeaderField: "X-Nola-Mobile-Nonce"),
              let digest = request.value(forHTTPHeaderField: "X-Nola-Mobile-Body-SHA256"),
              let proof = request.value(forHTTPHeaderField: "X-Nola-Mobile-Auth"),
              let signature = try? NolaTailcatGrant.decode(proof) else { return false }
        let path = url.path + (url.query.map { "?\($0)" } ?? "")
        let message = "v1\n\((request.httpMethod ?? "GET").uppercased())\n\(path)\n\(timestamp)\n\(nonce)\n\(digest)"
        return HMAC<SHA256>.isValidAuthenticationCode(signature, authenticating: Data(message.utf8), using: SymmetricKey(data: Data(configuration.token.utf8)))
    }
    static func sessionConfiguration() -> URLSessionConfiguration {
        let configuration = URLSessionConfiguration.default
        configuration.protocolClasses = [NolaTailcatURLProtocol.self] + (configuration.protocolClasses ?? [])
        return configuration
    }
    static let session = URLSession(configuration: sessionConfiguration())
}

private actor NolaTailcatConnections {
    static let shared = NolaTailcatConnections()
    private var parked = false
    #if canImport(NolaTailcat)
    private final class Native: @unchecked Sendable {
        let session = NolaTailcatNolatailcatNewSession()!
        let configuration: NolaTailcatTransport.Configuration
        private let lock = NSLock()
        private var lease: String?
        private var connected = false
        var ready: Bool { lock.withLock { connected } }
        func markReady() { lock.withLock { connected = true } }
        init(_ configuration: NolaTailcatTransport.Configuration) { self.configuration = configuration }
        func own(_ lease: String) { lock.withLock { self.lease = lease } }
        func close() async {
            _ = try? await Task.detached { try self.session.close() }.value
            let previous = lock.withLock { let value = lease; lease = nil; return value }
            if let previous { _ = try? await NolaTailcatGrant.exchange(configuration, payload: ["operation": "close", "leaseId": previous]) }
        }
    }
    private struct Entry {
        let id: UUID
        let configuration: NolaTailcatTransport.Configuration
        let native: Native
        let task: Task<(URL, Date), Error>
    }
    private var entries: [String: Entry] = [:]
    #endif
    func park() async {
        parked = true
        #if canImport(NolaTailcat)
        // Keep established ephemeral identity/grant in memory for foreground
        // return on another network. OS suspension is not background execution.
        let pending = entries.filter { !$0.value.native.ready }
        for (key, entry) in pending { entries.removeValue(forKey: key); entry.task.cancel() }
        for entry in pending.values { await entry.native.close() }
        #endif
    }
    func resume() { parked = false }
    func remove(_ key: String, generation: UUID) async {
        #if canImport(NolaTailcat)
        if let entry = entries[key], entry.configuration.generation == generation {
            entries.removeValue(forKey: key); entry.task.cancel(); await entry.native.close()
        }
        #endif
    }
    func suspend() async {
        #if canImport(NolaTailcat)
        let previous = entries; entries.removeAll()
        for entry in previous.values { entry.task.cancel(); await entry.native.close() }
        #endif
    }
    func endpoint(for configuration: NolaTailcatTransport.Configuration) async throws -> URL {
        guard !parked else { throw URLError(.backgroundSessionWasDisconnected) }
        #if canImport(NolaTailcat)
        let key = configuration.origin.absoluteString
        if let entry = entries[key] {
            let result = try await entry.task.value
            guard entries[key]?.id == entry.id else { throw URLError(.cancelled) }
            if entry.configuration == configuration, result.1.timeIntervalSinceNow > 30 { return result.0 }
            entries.removeValue(forKey: key); await entry.native.close()
        }
        let id = UUID()
        let native = Native(configuration)
        let task = Task<(URL, Date), Error> {
            do {
                let inspect = configuration.origin.appendingPathComponent("api/web/session")
                var check = URLRequest(url: inspect); check.timeoutInterval = 8
                let (statusData, statusResponse) = try await URLSession.shared.data(for: check)
                guard (statusResponse as? HTTPURLResponse)?.statusCode == 200,
                      let status = try JSONSerialization.jsonObject(with: statusData) as? [String: Any],
                      status["hostId"] as? String == configuration.hostID else { throw URLError(.badServerResponse) }
                guard let advertised = status["tailcatAvailable"] as? Bool else { throw URLError(.badServerResponse) }
                if !advertised { return (configuration.origin, .distantFuture) }
                let requestedAt = Date()
                let grant = try await NolaTailcatGrant.exchange(configuration, payload: [
                    "operation": "connect", "expectedHostId": configuration.hostID,
                    "clientPublicKey": native.session.publicKey()
                ])
                guard grant.hostId == configuration.hostID,
                      let address = grant.address,
                      let lease = grant.leaseId, !lease.isEmpty,
                      let seconds = grant.expiresIn, (31...1200).contains(seconds) else { throw URLError(.badServerResponse) }
                native.own(lease)
                let local = try await Task.detached {
                    var failure: NSError?
                    let value = native.session.connect(address, expectedHostID: configuration.hostID, relay: "", timeoutMillis: 30000, error: &failure)
                    if let failure { throw failure }
                    guard let url = URL(string: value), url.host == "127.0.0.1", url.scheme == "http" else { throw URLError(.badServerResponse) }
                    return url
                }.value
                try Task.checkCancellation()
                native.markReady()
                return (local, requestedAt.addingTimeInterval(seconds))
            } catch { await native.close(); throw error }
        }
        entries[key] = Entry(id: id, configuration: configuration, native: native, task: task)
        do { return try await task.value.0 }
        catch { if entries[key]?.id == id { entries.removeValue(forKey: key) }; throw error }
        #else
        return configuration.origin
        #endif
    }
}

// Reuses Nola's encrypted mobile envelope and proof contract. No bearer token,
// grant address, temporary key or response is logged or persisted.
private nonisolated struct NolaTailcatGrantReceipt: Decodable, Sendable {
    let hostId: String?
    let address: String?
    let leaseId: String?
    let expiresIn: Double?
    let closed: Bool?
    let closing: Bool?
}
private nonisolated enum NolaTailcatGrant {
    static func base64(_ value: Data) -> String {
        value.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }
    static func decode(_ value: String) throws -> Data {
        var value = value.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        value += String(repeating: "=", count: (4 - value.count % 4) % 4)
        guard let data = Data(base64Encoded: value) else { throw URLError(.cannotDecodeContentData) }; return data
    }
    static func exchange(_ configuration: NolaTailcatTransport.Configuration, payload: [String: String]) async throws -> NolaTailcatGrantReceipt {
        let key = SymmetricKey(data: Data(SHA256.hash(data: Data(configuration.token.utf8))))
        let aad = Data("nola-mobile-bridge-v1".utf8)
        let sealed = try AES.GCM.seal(JSONSerialization.data(withJSONObject: payload), using: key, authenticating: aad)
        let body = try JSONSerialization.data(withJSONObject: ["v": 1, "alg": "AES-256-GCM", "nonce": base64(Data(sealed.nonce)), "ciphertext": base64(sealed.ciphertext), "tag": base64(sealed.tag)])
        var request = URLRequest(url: configuration.origin.appendingPathComponent("api/mobile/transport/tailcat"))
        request.httpMethod = "POST"; request.httpBody = body; request.timeoutInterval = payload["operation"] == "close" ? 6 : 40
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let timestamp = String(UInt64(Date().timeIntervalSince1970)), nonce = UUID().uuidString.lowercased()
        let digest = SHA256.hash(data: body).map { String(format: "%02x", $0) }.joined()
        let message = "v1\nPOST\n\(request.url!.path)\n\(timestamp)\n\(nonce)\n\(digest)"
        request.setValue(timestamp, forHTTPHeaderField: "X-Nola-Mobile-Timestamp")
        request.setValue(nonce, forHTTPHeaderField: "X-Nola-Mobile-Nonce")
        request.setValue(digest, forHTTPHeaderField: "X-Nola-Mobile-Body-SHA256")
        request.setValue(base64(Data(HMAC<SHA256>.authenticationCode(for: Data(message.utf8), using: SymmetricKey(data: Data(configuration.token.utf8))))), forHTTPHeaderField: "X-Nola-Mobile-Auth")
        let (data, response) = try await URLSession.shared.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200, data.count <= 65536,
              let envelope = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              envelope["v"] as? Int == 1, envelope["alg"] as? String == "AES-256-GCM",
              let nonce = envelope["nonce"] as? String, let ciphertext = envelope["ciphertext"] as? String,
              let tag = envelope["tag"] as? String else { throw URLError(.badServerResponse) }
        let box = try AES.GCM.SealedBox(nonce: AES.GCM.Nonce(data: decode(nonce)), ciphertext: decode(ciphertext), tag: decode(tag))
        let plain = try AES.GCM.open(box, using: key, authenticating: aad)
        return try JSONDecoder().decode(NolaTailcatGrantReceipt.self, from: plain)
    }
}

private final class NolaTailcatURLProtocol: URLProtocol, URLSessionDataDelegate, @unchecked Sendable {
    private let stateLock = NSLock()
    private var stopped = false
    private var preparing: Task<Void, Never>?
    private var loading: URLSessionDataTask?
    private var session: URLSession?
    override class func canInit(with request: URLRequest) -> Bool {
        guard let url = request.url, let configuration = NolaTailcatTransport.configuration(for: url) else { return false }
        return NolaTailcatTransport.matches(request, configuration: configuration)
    }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        preparing = Task {
            do {
                guard let original = request.url, let configuration = NolaTailcatTransport.configuration(for: original) else { throw URLError(.badURL) }
                guard NolaTailcatTransport.matches(request, configuration: configuration) else { throw URLError(.cancelled) }
                let endpoint = try await NolaTailcatConnections.shared.endpoint(for: configuration)
                var parts = URLComponents(url: original, resolvingAgainstBaseURL: false)!
                parts.scheme = endpoint.scheme; parts.host = endpoint.host; parts.port = endpoint.port
                guard let url = parts.url else { throw URLError(.badURL) }
                var forwarded = request; forwarded.url = url
                try Task.checkCancellation()
                let session = URLSession(configuration: .default, delegate: self, delegateQueue: nil)
                stateLock.withLock {
                    if stopped { session.invalidateAndCancel(); return }
                    self.session = session; loading = session.dataTask(with: forwarded); loading?.resume()
                }
            } catch { client?.urlProtocol(self, didFailWithError: error) }
        }
    }
    override func stopLoading() { stateLock.withLock { stopped = true; preparing?.cancel(); loading?.cancel(); session?.invalidateAndCancel() } }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        if let http = response as? HTTPURLResponse, let original = request.url,
           let remapped = HTTPURLResponse(url: original, statusCode: http.statusCode, httpVersion: nil, headerFields: http.allHeaderFields as? [String: String]) {
            client?.urlProtocol(self, didReceive: remapped, cacheStoragePolicy: .notAllowed)
        } else { client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed) }
        completionHandler(.allow)
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) { client?.urlProtocol(self, didLoad: data) }
    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if let error { client?.urlProtocol(self, didFailWithError: error) } else { client?.urlProtocolDidFinishLoading(self) }
        session.finishTasksAndInvalidate()
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        // Never forward signed mobile credentials to a different origin.
        completionHandler(nil)
    }
}
