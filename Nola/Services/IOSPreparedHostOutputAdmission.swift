// SPDX-License-Identifier: Apache-2.0
// Non-secret execution facts for the shared output admission boundary.
import Foundation
import CryptoKit

@MainActor
enum IOSPreparedHostOutputAdmission {
    static func httpRoute(configuration: OpenAIProvider.Config, provider: String) throws -> AgentCoreJSONValue {
        guard var url = URLComponents(url: configuration.baseURL, resolvingAgainstBaseURL: false),
              let scheme = url.scheme?.lowercased(), ["http", "https"].contains(scheme),
              let host = url.host, !host.isEmpty,
              url.user == nil, url.password == nil, url.fragment == nil else {
            throw IOSApplicationRuntimeError.rejected("Invalid non-secret model route")
        }
        let query = url.percentEncodedQuery
        url.percentEncodedQuery = nil
        url.scheme = scheme
        url.host = host.lowercased()
        if (scheme == "http" && url.port == 80) || (scheme == "https" && url.port == 443) { url.port = nil }
        if url.percentEncodedPath.isEmpty { url.percentEncodedPath = "/" }
        guard let endpoint = url.string else { throw IOSApplicationRuntimeError.invalidResponse }
        let wireProtocol: String
        let field: String
        switch configuration.wireProtocol {
        case .chatCompletions:
            wireProtocol = "chat_completions"
            field = (configuration.executionCapabilities.chatOutputTokenField ?? .auto)
                .requestKey(baseURL: configuration.baseURL)
        case .responses:
            wireProtocol = "responses"
            field = "max_output_tokens"
        default:
            throw IOSApplicationRuntimeError.rejected("Unsupported local-host model protocol")
        }
        // Core verifies canonical URL spelling. Do not decode/re-encode path
        // escapes to imitate WHATWG normalization or conceal a different route.
        return .object([
            "kind": .string("http"), "protocol": .string(wireProtocol), "siwc_account": .null,
            "output_token_field": .string(field), "provider": .string(provider),
            "endpoint": .string(endpoint), "model": .string(configuration.model),
            "endpoint_query_sha256": query.map { .string(sha($0)) } ?? .null,
            "endpoint_query_source": source(query != nil),
            "credential_sha256": .string(sha(configuration.apiKey)),
            "credential_source": source(!configuration.apiKey.isEmpty),
        ])
    }

    static func nativeRoute(modelID: String) -> AgentCoreJSONValue {
        .object(["kind": .string("native"), "adapter": .string("mlx"),
            "adapter_contract_revision": .number(1), "model_id": .string(modelID), "profile_ref": .null])
    }

    static func admit(route: AgentCoreJSONValue, outputTokens: Int,
        using runtime: IOSApplicationRuntime? = IOSApplicationRuntime.shared) throws -> AgentCoreJSONValue {
        guard let runtime else { throw IOSApplicationRuntimeError.applicationUnavailable }
        guard outputTokens > 0 else { throw IOSApplicationRuntimeError.invalidResponse }
        let limits: AgentCoreJSONValue = .object(["route": route,
            "max_input_tokens": .null, "max_output_tokens": .null, "max_total_tokens": .null])
        let preference: AgentCoreJSONValue = .object(["explicit": .number(Double(outputTokens))])
        let admitted = try runtime.admitHostOutput(routes: [limits], preference: preference)
        try requireExact(admitted, route: route, outputTokens: outputTokens)
        return admitted
    }

    static func requireExact(_ admission: AgentCoreJSONValue, route: AgentCoreJSONValue, outputTokens: Int) throws {
        guard case .object(let object) = admission,
              case .object(let resolved)? = object["resolved"],
              resolved["tokens"] == .number(Double(outputTokens)),
              case .array(let routes)? = object["routes"], routes.count == 1,
              case .object(let limits) = routes[0], limits["route"] == route else {
            throw IOSApplicationRuntimeError.rejected("Model output admission does not match execution")
        }
    }

    private static func source(_ present: Bool) -> AgentCoreJSONValue {
        .object(["kind": .string(present ? "ephemeral" : "none")])
    }

    private static func sha(_ value: String) -> String {
        SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined()
    }
}
