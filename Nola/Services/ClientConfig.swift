// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ClientConfig logic in the iOS service layer.
// - Interfaces:
//   - ClientConfig
// - APIs:
//   - none
//
import Foundation
import Combine

/// Centralized client-side configuration persisted in UserDefaults.
public final class ClientConfig: ObservableObject {
    public static let shared = ClientConfig()
    public static var kHardcodedOfficialAPIBase_CN = "https://api-inference.modelscope.cn"
    public static var kHardcodedOfficialAPIBase_Global = "https://api-inference.modelscope.cn"

    private let defaults = UserDefaults.standard

    private let pkURLLimit = "clientconfig.url_limit"

    @Published public private(set) var urlSourcesLimit: Int

    private init() {
        let defaultURLs = 10

        if let persistedURLObj = defaults.object(forKey: pkURLLimit) as? Int {
            self.urlSourcesLimit = persistedURLObj
        } else {
            self.urlSourcesLimit = defaultURLs
        }
    }

    public func updateURLSourcesLimit(_ limit: Int) {
        DispatchQueue.main.async {
            self.urlSourcesLimit = max(0, limit)
            self.defaults.set(self.urlSourcesLimit, forKey: self.pkURLLimit)
        }
    }

    public func effectiveURLLimit() -> Int { urlSourcesLimit }
}
