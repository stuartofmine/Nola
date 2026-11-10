// SPDX-License-Identifier: MIT
// Out-of-process MLX inference runtime for NolaMac.

import Darwin
import Foundation
import MLX
import MLXLMCommon
import MLXLLM
import Network
import Tokenizers

private struct RuntimeConfiguration: Decodable, Sendable {
    let parentPID: Int32
    let token: String
    let modelID: String
    let displayName: String
    let installedDirectory: String?
    let estimatedRuntimeBytes: Int64
    let contextWindowTokens: Int
    let maxOutputTokens: Int
    let temperature: Double
    let topP: Double
    let topK: Int
    let repetitionPenalty: Double
    let presencePenalty: Double
    let frequencyPenalty: Double
}

private struct RuntimeEvent: Encodable {
    let phase: String
    let port: UInt16?
    let error: String?
}

private struct ModelToolCall: Decodable, Sendable {
    struct Function: Decodable, Sendable {
        let name: String
        let arguments: String
    }
    let function: Function
}

private struct ModelMessage: Decodable, Sendable {
    let role: String
    let content: String
    let toolCalls: [ModelToolCall]

    private enum CodingKeys: String, CodingKey {
        case role, content
        case toolCalls = "tool_calls"
    }

    init(from decoder: any Swift.Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        role = try container.decode(String.self, forKey: .role)
        content = try container.decodeIfPresent(String.self, forKey: .content) ?? ""
        toolCalls = try container.decodeIfPresent([ModelToolCall].self, forKey: .toolCalls) ?? []
    }

    var localInferenceContent: String {
        guard role == "assistant", !toolCalls.isEmpty else { return content }
        let calls = toolCalls.map { call in
            let arguments: Any = call.function.arguments.data(using: .utf8)
                .flatMap { try? JSONSerialization.jsonObject(with: $0) }
                ?? call.function.arguments
            let payload: [String: Any] = ["name": call.function.name, "arguments": arguments]
            let data = try? JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys])
            let json = data.flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
            return "<tool_call>\n\(json)\n</tool_call>"
        }
        return ([content] + calls).filter { !$0.isEmpty }.joined(separator: "\n")
    }
}

private struct ChatRequest: Decodable, Sendable {
    let model: String
    let messages: [ModelMessage]
    let tools: [Tool]?
    let stream: MLXStreamRequest
    let generation: MLXGenerationRequest
    private enum CodingKeys: String, CodingKey {
        case model, messages, tools
    }

    init(from decoder: any Swift.Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        model = try values.decode(String.self, forKey: .model)
        messages = try values.decode([ModelMessage].self, forKey: .messages)
        tools = try values.decodeIfPresent([Tool].self, forKey: .tools)
        stream = try MLXStreamRequest(from: decoder)
        generation = try MLXGenerationRequest(from: decoder)
    }

    struct Tool: Decodable, Sendable {
        let type: String
        let function: Function

        struct Function: Decodable, Sendable {
            let name: String
            let description: String?
            let parameters: SendableJSON?
        }

        var toolSpec: ToolSpec {
            [
                "type": type,
                "function": [
                    "name": function.name,
                    "description": function.description ?? "",
                    "parameters": function.parameters?.value ?? ["type": "object"],
                ] as [String: any Sendable],
            ]
        }
    }
}

private indirect enum SendableJSON: Decodable, Sendable {
    case object([String: SendableJSON])
    case array([SendableJSON])
    case string(String)
    case number(Double)
    case bool(Bool)
    case null

    init(from decoder: any Swift.Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() { self = .null }
        else if let value = try? container.decode([String: SendableJSON].self) { self = .object(value) }
        else if let value = try? container.decode([SendableJSON].self) { self = .array(value) }
        else if let value = try? container.decode(Bool.self) { self = .bool(value) }
        else if let value = try? container.decode(Double.self) { self = .number(value) }
        else { self = .string(try container.decode(String.self)) }
    }

    var value: any Sendable {
        switch self {
        case .object(let object): return object.mapValues(\.value)
        case .array(let array): return array.map(\.value)
        case .string(let string): return string
        case .number(let number): return number
        case .bool(let bool): return bool
        case .null: return NSNull()
        }
    }
}

private struct HTTPRequest {
    let method: String
    let path: String
    let headers: [String: String]
    let body: Data

    static func parse(_ data: Data) -> HTTPRequest? {
        let separator = Data("\r\n\r\n".utf8)
        guard let headerRange = data.range(of: separator),
              let headerText = String(data: data[..<headerRange.lowerBound], encoding: .utf8) else {
            return nil
        }
        let lines = headerText.components(separatedBy: "\r\n")
        let requestLine = lines.first?.split(separator: " ") ?? []
        guard requestLine.count >= 2 else { return nil }
        var headers: [String: String] = [:]
        for line in lines.dropFirst() {
            let parts = line.split(separator: ":", maxSplits: 1)
            if parts.count == 2 {
                headers[String(parts[0]).lowercased()] = parts[1].trimmingCharacters(in: .whitespaces)
            }
        }
        let bodyStart = headerRange.upperBound
        let length = Int(headers["content-length"] ?? "0") ?? 0
        guard data.count >= bodyStart + length else { return nil }
        return HTTPRequest(
            method: String(requestLine[0]),
            path: String(requestLine[1]),
            headers: headers,
            body: data.subdata(in: bodyStart..<(bodyStart + length))
        )
    }
}

private actor InferenceEngine {
    private let configuration: RuntimeConfiguration
    private var container: ModelContainer?

    init(configuration: RuntimeConfiguration) {
        self.configuration = configuration
    }

    func prepare() async throws {
        _ = try await modelContainer()
    }

    func stream(_ request: ChatRequest) async throws -> AsyncThrowingStream<Generation, Error> {
        guard request.model == configuration.modelID else {
            throw RuntimeFailure.invalidRequest("Requested model does not match the active runtime")
        }
        guard !request.messages.isEmpty,
              request.messages.contains(where: { $0.role != "system" }) else {
            throw RuntimeFailure.invalidRequest("A chat message is required")
        }
        let supportedRoles = Set(["system", "user", "assistant", "tool"])
        guard request.messages.allSatisfy({ supportedRoles.contains($0.role) }) else {
            throw RuntimeFailure.invalidRequest("Unsupported chat role")
        }
        let systemIndices = request.messages.indices.filter { request.messages[$0].role == "system" }
        guard systemIndices.count <= 1,
              systemIndices.first.map({ $0 == request.messages.startIndex }) ?? true else {
            throw RuntimeFailure.invalidRequest("The system message must be first")
        }
        let messages = request.messages.compactMap { message in
            Chat.Message.Role(rawValue: message.role).map {
                Chat.Message(role: $0, content: message.localInferenceContent)
            }
        }
        guard let latest = messages.last else {
            throw RuntimeFailure.invalidRequest("A chat message is required")
        }
        let generation = try request.generation.resolve(defaults: MLXGenerationParameters(
            maxTokens: configuration.maxOutputTokens,
            temperature: configuration.temperature, topP: configuration.topP, topK: configuration.topK,
            repetitionPenalty: configuration.repetitionPenalty, presencePenalty: configuration.presencePenalty,
            frequencyPenalty: configuration.frequencyPenalty))
        let parameters = GenerateParameters(
            maxTokens: generation.maxTokens,
            maxKVSize: configuration.contextWindowTokens,
            temperature: Float(generation.temperature),
            topP: Float(generation.topP),
            topK: generation.topK,
            repetitionPenalty: Float(generation.repetitionPenalty),
            presencePenalty: Float(generation.presencePenalty),
            frequencyPenalty: Float(generation.frequencyPenalty)
        )
        let session = ChatSession(
            try await modelContainer(),
            instructions: nil,
            history: Array(messages.dropLast()),
            generateParameters: parameters,
            tools: request.tools?.map(\.toolSpec)
        )
        return session.streamDetails(to: latest.content, role: latest.role, images: [], videos: [])
    }

    private func modelContainer() async throws -> ModelContainer {
        if let container { return container }
        Memory.cacheLimit = configuration.estimatedRuntimeBytes >= 6_000_000_000
            ? 1_024 * 1_024 * 1_024
            : 512 * 1_024 * 1_024
        let modelConfiguration: ModelConfiguration
        if let directory = configuration.installedDirectory {
            modelConfiguration = ModelConfiguration(directory: URL(fileURLWithPath: directory))
        } else {
            modelConfiguration = ModelConfiguration(id: configuration.modelID)
        }
        let loaded = try await LLMModelFactory.shared.loadContainer(configuration: modelConfiguration)
        container = loaded
        return loaded
    }
}

private final class RuntimeServer: @unchecked Sendable {
    private let configuration: RuntimeConfiguration
    private let engine: InferenceEngine
    private let queue = DispatchQueue(label: "ai.nola.mlx-runtime", qos: .userInitiated)
    private var listener: NWListener?
    private var parentMonitor: DispatchSourceTimer?

    init(configuration: RuntimeConfiguration) {
        self.configuration = configuration
        engine = InferenceEngine(configuration: configuration)
    }

    func run() async throws {
        let port = try startListener()
        emit(.init(phase: "loading_weights", port: port.rawValue, error: nil))
        do {
            try await engine.prepare()
        } catch {
            emit(.init(phase: "failed", port: port.rawValue, error: error.localizedDescription))
            throw error
        }
        emit(.init(phase: "ready", port: port.rawValue, error: nil))
        monitorParent()
        // The runtime lifetime is owned by the supervisor process. It ends via
        // SIGTERM/SIGKILL or the parent-PID monitor, not by completing this task.
        await withUnsafeContinuation { (_: UnsafeContinuation<Void, Never>) in }
    }

    private func startListener() throws -> NWEndpoint.Port {
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
        let listener = try NWListener(using: parameters)
        listener.newConnectionHandler = { [weak self] connection in self?.accept(connection) }
        listener.start(queue: queue)
        let deadline = Date().addingTimeInterval(2)
        while (listener.port?.rawValue ?? 0) == 0, Date() < deadline {
            Thread.sleep(forTimeInterval: 0.01)
        }
        guard let port = listener.port, port.rawValue != 0 else {
            listener.cancel()
            throw RuntimeFailure.serverStartFailed
        }
        self.listener = listener
        return port
    }

    private func monitorParent() {
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + 1, repeating: 1)
        timer.setEventHandler { [parentPID = configuration.parentPID] in
            if Darwin.kill(parentPID, 0) != 0 { exit(0) }
        }
        parentMonitor = timer
        timer.resume()
    }

    private func accept(_ connection: NWConnection) {
        connection.start(queue: queue)
        receive(on: connection, buffer: Data())
    }

    private func receive(on connection: NWConnection, buffer: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 1_048_576) {
            [weak self] data, _, isComplete, error in
            guard let self else { return }
            var next = buffer
            if let data { next.append(data) }
            if let request = HTTPRequest.parse(next) {
                Task { await self.handle(request, connection: connection) }
            } else if isComplete || error != nil || next.count > 8 * 1_048_576 {
                connection.cancel()
            } else {
                self.receive(on: connection, buffer: next)
            }
        }
    }

    private func handle(_ request: HTTPRequest, connection: NWConnection) async {
        let writer = MLXHTTPResponseWriter { data in
            try await self.send(data, on: connection)
        }
        defer { connection.cancel() }
        do {
            guard request.headers["authorization"] == "Bearer \(configuration.token)" else {
                try await writer.json(["error": ["message": "Unauthorized"]], status: 401)
                return
            }
            if request.method == "GET", request.path == "/v1/models" {
                try await writer.json(["object": "list", "data": [[
                    "id": configuration.modelID,
                    "object": "model",
                    "owned_by": "nola-local",
                    "context_window": configuration.contextWindowTokens,
                ]]])
                return
            }
            guard request.method == "POST", request.path == "/v1/chat/completions" else {
                try await writer.json(["error": ["message": "Not found"]], status: 404)
                return
            }
            let chat = try JSONDecoder().decode(ChatRequest.self, from: request.body)
            let stream = try await engine.stream(chat)
            var response = MLXChatResponse(model: configuration.modelID)
            var info: GenerateCompletionInfo?
            if !chat.stream.enabled {
                var text = ""
                var calls: [[String: Any]] = []
                for try await item in stream {
                    if let chunk = item.chunk { text += chunk }
                    if let call = item.toolCall { calls.append(try toolCallJSON(call, response: &response)) }
                    if let completed = item.info { info = completed }
                }
                try await writer.json(response.completion(text: text, calls: calls,
                    reason: finishReason(info), promptTokens: info?.promptTokenCount,
                    completionTokens: info?.generationTokenCount))
                return
            }
            try await writer.startStream()
            try await writer.event(response.chunk(delta: ["role": "assistant", "content": ""]))
            for try await item in stream {
                if let chunk = item.chunk, !chunk.isEmpty {
                    try await writer.event(response.chunk(delta: ["content": chunk]))
                }
                if let call = item.toolCall {
                    let encoded = try toolCallJSON(call, response: &response)
                    try await writer.event(response.chunk(delta: ["tool_calls": [encoded]]))
                }
                if let completed = item.info { info = completed }
            }
            for chunk in try response.finishStream(reason: finishReason(info),
                promptTokens: info?.promptTokenCount, completionTokens: info?.generationTokenCount,
                includeUsage: chat.stream.includeUsage) {
                try await writer.event(chunk)
            }
            try await writer.finishStream()
        } catch {
            await writer.fail(error)
        }
    }

    private func finishReason(_ info: GenerateCompletionInfo?) -> MLXChatResponse.Finish? {
        guard let info else { return nil }
        switch info.stopReason {
        case .stop: return .stop
        case .length: return .length
        case .cancelled: return .cancelled
        }
    }

    private func toolCallJSON(_ call: ToolCall, response: inout MLXChatResponse) throws -> [String: Any] {
        let arguments = call.function.arguments.mapValues(\.anyValue)
        let data = try JSONSerialization.data(withJSONObject: arguments, options: [.sortedKeys])
        return try response.tool(name: call.function.name, arguments: String(decoding: data, as: UTF8.self))
    }

    private func send(_ data: Data, on connection: NWConnection) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            connection.send(content: data, completion: .contentProcessed { error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume() }
            })
        }
    }

    private func emit(_ event: RuntimeEvent) {
        guard let data = try? JSONEncoder().encode(event) else { return }
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data("\n".utf8))
    }
}

private enum RuntimeFailure: LocalizedError {
    case serverStartFailed
    case invalidRequest(String)

    var errorDescription: String? {
        switch self {
        case .serverStartFailed: return "Unable to bind the local model service"
        case .invalidRequest(let detail): return detail
        }
    }
}

do {
    let data = FileHandle.standardInput.readDataToEndOfFile()
    let configuration = try JSONDecoder().decode(RuntimeConfiguration.self, from: data)
    guard configuration.parentPID > 1, Darwin.kill(configuration.parentPID, 0) == 0 else {
        throw RuntimeFailure.invalidRequest("Parent process is unavailable")
    }
    let server = RuntimeServer(configuration: configuration)
    try await server.run()
} catch {
    let event = RuntimeEvent(phase: "failed", port: nil, error: error.localizedDescription)
    if let data = try? JSONEncoder().encode(event) {
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data("\n".utf8))
    }
    exit(1)
}
