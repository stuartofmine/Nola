import Foundation
import CryptoKit
import Testing
@testable import Nola

@MainActor
struct IOSPreparedHostOutputAdmissionTests {
    @Test
    func exactEncodedQueryAndCommonURLNormalizationReachSharedAdmission() throws {
        try withRuntime { runtime in
            for (input, endpoint, query) in [
                ("HTTPS://EXAMPLE.COM:443", "https://example.com/", Optional<String>.none),
                ("http://EXAMPLE.COM:80/v1?", "http://example.com/v1", ""),
                ("https://example.com:8443/proxy%2Ftenant/v1?a=%2F+b&a=2", "https://example.com:8443/proxy%2Ftenant/v1", "a=%2F+b&a=2"),
                ("https://example.com/%25/v1", "https://example.com/%25/v1", nil)
            ] {
                let config = configuration(input)
                let route = try IOSPreparedHostOutputAdmission.httpRoute(configuration: config, provider: "fixture")
                guard case .object(let fields) = route else { Issue.record("Expected HTTP route"); return }
                #expect(fields["endpoint"] == .string(endpoint))
                #expect(fields["endpoint_query_sha256"] == (query.map { .string(sha($0)) } ?? .null))
                #expect(fields["endpoint_query_source"] == .object(["kind": .string(query == nil ? "none" : "ephemeral")]))
                let admitted = try IOSPreparedHostOutputAdmission.admit(route: route, outputTokens: 64, using: runtime)
                let raw = String(decoding: try JSONEncoder().encode(admitted), as: UTF8.self)
                #expect(!raw.contains("private-key"))
                #expect(!raw.contains("a=%2F+b"))
            }
        }
    }

    @Test
    func malformedOrNonCanonicalRoutesFailWithoutDisclosingMaterials() throws {
        try withRuntime { runtime in
            for input in ["https://user:private-password@example.com/v1", "https://example.com/v1#private-fragment",
                          "file:///private-model", "https://example.com/a/../v1"] {
                do {
                    let route = try IOSPreparedHostOutputAdmission.httpRoute(configuration: configuration(input), provider: "fixture")
                    _ = try IOSPreparedHostOutputAdmission.admit(route: route, outputTokens: 64, using: runtime)
                    Issue.record("Expected route rejection")
                } catch {
                    let message = String(describing: error)
                    #expect(!message.contains("private-password"))
                    #expect(!message.contains("private-fragment"))
                    #expect(!message.contains("private-key"))
                }
            }
        }
    }

    @Test
    func outputFieldUsesProviderRuleAndUnknownCapsStayUnknown() throws {
        try withRuntime { runtime in
            for wire in [ModelWireProtocol.chatCompletions, .responses] {
                var config = configuration("https://example.com/v1")
                config.wireProtocol = wire
                config.executionCapabilities.chatOutputTokenField = .maxCompletionTokens
                let route = try IOSPreparedHostOutputAdmission.httpRoute(configuration: config, provider: "fixture")
                guard case .object(let fields) = route else { Issue.record("Expected route"); return }
                #expect(fields["output_token_field"] == .string(wire == .responses ? "max_output_tokens" : "max_completion_tokens"))
                let admission = try IOSPreparedHostOutputAdmission.admit(route: route, outputTokens: 64, using: runtime)
                guard case .object(let object) = admission, case .array(let routes)? = object["routes"],
                      case .object(let limits) = routes[0] else { Issue.record("Expected limits"); return }
                #expect(limits["max_input_tokens"] == .null)
                #expect(limits["max_output_tokens"] == .null)
                #expect(limits["max_total_tokens"] == .null)
                #expect(throws: Error.self) {
                    try IOSPreparedHostOutputAdmission.requireExact(admission, route: route, outputTokens: 128)
                }
            }
        }
    }

    private func configuration(_ url: String) -> OpenAIProvider.Config {
        .init(baseURL: URL(string: url)!, apiKey: "private-key", model: "fixture", maxCompletionTokens: 64)
    }

    private func sha(_ value: String) -> String {
        SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    private func withRuntime(_ body: (IOSApplicationRuntime) throws -> Void) throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try body(IOSApplicationRuntime(databaseURL: root.appendingPathComponent("application.db")))
    }
}
