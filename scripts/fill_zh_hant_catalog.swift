#!/usr/bin/env swift
// SPDX-License-Identifier: Apache-2.0
// Fill missing Traditional Chinese catalog units from reviewed Simplified
// Chinese source units using ICU, then apply Nola terminology conventions.

import Foundation

let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
let catalogURL = root.appendingPathComponent("NolaMac/Localizable.xcstrings")
let data = try Data(contentsOf: catalogURL)
guard var catalog = try JSONSerialization.jsonObject(with: data) as? [String: Any],
      var strings = catalog["strings"] as? [String: Any] else {
    throw NSError(domain: "NolaLocalization", code: 1)
}

let terminology: [(String, String)] = [
    ("創建", "建立"),
    ("日曆", "行事曆"),
    ("提醒事項", "提醒事項"),
    ("數據源", "資料來源"),
    ("數據", "資料"),
    ("文件夾", "資料夾"),
    ("文件", "檔案"),
    ("應用程式", "App"),
    ("應用", "App"),
    ("設置", "設定"),
    ("搜索", "搜尋"),
    ("默認", "預設"),
    ("網絡", "網路"),
    ("智能體", "智慧代理"),
    ("信息", "資訊"),
    ("視頻", "影片"),
    ("鼠標", "滑鼠"),
]

var changed = 0
for (key, rawEntry) in strings {
    guard var entry = rawEntry as? [String: Any],
          var localizations = entry["localizations"] as? [String: Any],
          let source = (localizations["zh-Hans"] as? [String: Any])?["stringUnit"] as? [String: Any],
          let sourceValue = source["value"] as? String,
          !sourceValue.isEmpty else { continue }
    if let current = (localizations["zh-Hant"] as? [String: Any])?["stringUnit"] as? [String: Any],
       current["state"] as? String == "translated",
       !(current["value"] as? String ?? "").isEmpty {
        continue
    }
    var value = sourceValue.applyingTransform(StringTransform("Hans-Hant"), reverse: false) ?? sourceValue
    for (from, to) in terminology {
        value = value.replacingOccurrences(of: from, with: to)
    }
    localizations["zh-Hant"] = [
        "stringUnit": ["state": "translated", "value": value],
    ]
    entry["localizations"] = localizations
    strings[key] = entry
    changed += 1
}

catalog["strings"] = strings
let output = try JSONSerialization.data(withJSONObject: catalog, options: [.prettyPrinted, .sortedKeys])
try output.write(to: catalogURL, options: .atomic)
try Data("\n".utf8).append(to: catalogURL)
print("Filled \(changed) Traditional Chinese localization units.")

private extension Data {
    func append(to url: URL) throws {
        let handle = try FileHandle(forWritingTo: url)
        defer { try? handle.close() }
        try handle.seekToEnd()
        try handle.write(contentsOf: self)
    }
}
