// SPDX-License-Identifier: MIT
// Transport selection is decoded once, independently of model generation.
import Foundation

struct MLXStreamRequest: Decodable, Sendable {
    let enabled: Bool
    let includeUsage: Bool

    private enum CodingKeys: String, CodingKey {
        case stream
        case options = "stream_options"
    }
    private struct Options: Decodable {
        let includeUsage: Bool?
        enum CodingKeys: String, CodingKey { case includeUsage = "include_usage" }
    }

    init(from decoder: any Swift.Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        enabled = try values.decodeIfPresent(Bool.self, forKey: .stream) ?? false
        let options = try values.decodeIfPresent(Options.self, forKey: .options)
        includeUsage = enabled && options?.includeUsage == true
    }
}
