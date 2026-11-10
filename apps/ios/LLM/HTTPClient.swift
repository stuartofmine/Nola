// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements HTTPClient logic in the iOS LLM integration layer.
// - Interfaces:
//   - HTTPClient
//   - DefaultHTTPClient
//   - MockHTTPClient
// - APIs:
//   - none
//
//
//  HTTPClient.swift
//  Nola
//
//  Network abstraction for testability and dependency injection.
//

import Foundation

/// The iOS 17 iterator entry point requires a nonisolated conformance. Bind that
/// contract directly to the iterator, rather than an AsyncSequence associated
/// conformance. This does not require a mutable iterator instance to be Sendable.
nonisolated enum IOSHTTPByteIterator {
    static func next<Iterator: AsyncIteratorProtocol & SendableMetatype>(
        _ iterator: inout Iterator
    ) async throws -> UInt8? where Iterator.Element == UInt8 {
        try await iterator.next()
    }
}

// Access Keychain helper to read stored UUID

/// Protocol for performing HTTP requests; allows mocking in tests.
public protocol HTTPClient {
    func perform(_ request: URLRequest) async throws -> (Data, HTTPURLResponse)
}

/// Default implementation using URLSession.
public final class DefaultHTTPClient: HTTPClient {
    private let session: URLSession
    
    public init(session: URLSession? = nil) {
        if let session {
            self.session = session
            return
        }
        let config = URLSessionConfiguration.default
        config.waitsForConnectivity = true
        config.timeoutIntervalForRequest = 40
        config.timeoutIntervalForResource = 90
        config.httpMaximumConnectionsPerHost = 4
        self.session = URLSession(configuration: config)
    }
    
    public func perform(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        var req = request
        // Inject X-Client-UUID header if available in Keychain
        if let uuid = KeychainHelper.shared.read("com.nola.client_uuid") {
            req.setValue(uuid, forHTTPHeaderField: "X-Client-UUID")
            // Temporary debug/info: log that we injected the client UUID
            Logger().log(.info, "HTTPClient", "Injecting X-Client-UUID", metadata: ["uuid": uuid])
        } else {
            // Temporary debug: explicitly log absence of client UUID
            Logger().log(.info, "HTTPClient", "No X-Client-UUID found in Keychain", metadata: [:])
        }
        let (data, response) = try await session.data(for: req)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw URLError(.badServerResponse)
        }
        return (data, httpResponse)
    }
}

/// Mock HTTP client for testing; returns pre-configured responses.
public final class MockHTTPClient: HTTPClient {
    public var responses: [(Data, HTTPURLResponse)] = []
    public var errors: [Error] = []
    private var callIndex = 0
    
    public init() {}
    
    public func perform(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        defer { callIndex += 1 }
        
        if callIndex < errors.count {
            throw errors[callIndex]
        }
        
        if callIndex < responses.count {
            return responses[callIndex]
        }
        
        // Default: return empty 200 response
        let url = request.url ?? URL(fileURLWithPath: "/")
        guard let response = HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil) else {
            throw URLError(.badServerResponse)
        }
        return (Data(), response)
    }
}
