// SPDX-License-Identifier: MIT
import Foundation

@main
struct StreamRequestTests {
    static func decode(_ json: String) throws -> MLXStreamRequest {
        try JSONDecoder().decode(MLXStreamRequest.self, from: Data(json.utf8))
    }

    static func main() throws {
        for json in ["{}", "{\"stream\":null}", "{\"stream\":false}",
                     "{\"stream_options\":{\"include_usage\":true}}"] {
            let request = try decode(json)
            precondition(!request.enabled && !request.includeUsage)
        }
        let basic = try decode("{\"stream\":true}")
        precondition(basic.enabled && !basic.includeUsage)
        let usage = try decode("{\"stream\":true,\"stream_options\":{\"include_usage\":true}}")
        precondition(usage.enabled && usage.includeUsage)
        let disabled = try decode("{\"stream\":true,\"stream_options\":{\"include_usage\":false}}")
        precondition(disabled.enabled && !disabled.includeUsage)
        for json in ["{\"stream\":1}", "{\"stream\":\"true\"}",
                     "{\"stream_options\":[]}", "{\"stream_options\":{\"include_usage\":1}}"] {
            do { _ = try decode(json); fatalError("Invalid stream control accepted") }
            catch { }
        }
        print("MLX stream request contracts passed: non-stream default, explicit stream and strict option types")
    }
}
