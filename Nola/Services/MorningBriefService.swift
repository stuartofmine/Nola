// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements MorningBriefService logic in the iOS service layer.
// - Interfaces:
//   - BriefItem
//   - MorningBriefLocal
//   - MorningBriefFormatter
// - APIs:
//   - none
//
//
//  MorningBriefService.swift
//  Nola
//
//  Service for generating morning and evening summary briefs.
//

import Foundation

public struct BriefItem: Codable {
    public let text: String
    public let priority: Double
    public let dedupeKey: String
    public let actionType: String?
    
    public init(text: String, priority: Double, dedupeKey: String, actionType: String? = nil) {
        self.text = text
        self.priority = priority
        self.dedupeKey = dedupeKey
        self.actionType = actionType
    }
}

// Local MorningBrief definition for MorningBriefService
struct MorningBriefLocal: Codable {
    let id: String
    let date: String
    let items: [BriefItem]
    let explain: String?
    let generatedBy: String?
}

// MARK: - Brief Formatting

public final class MorningBriefFormatter {
    public static func formatAsText(_ briefItems: [BriefItem], explain: String? = nil) -> String {
        let header = NSLocalizedString("autogen.MorningBriefService.b053fef8", comment: "AUTOGEN")
        var text = "\(header)\n\n"
        
        for (index, item) in briefItems.enumerated() {
            text += "\(index + 1). \(item.text)\n"
        }
        
        if let explain = explain {
            text += "\n💡 \(explain)"
        }
        
        return text
    }
    
    public static func formatAsHTML(_ briefItems: [BriefItem], explain: String? = nil) -> String {
        let header = NSLocalizedString("autogen.MorningBriefService.15081b04", comment: "AUTOGEN")
        var html = """
        <html>
        <head>
            <meta charset="utf-8">
            <style>
                body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI"; margin: 20px; }
                h1 { color: #333; }
                ol { line-height: 1.8; }
                li { margin-bottom: 12px; }
                .explain { color: #666; font-size: 14px; margin-top: 20px; }
            </style>
        </head>
        <body>
        <h1>\(header)</h1>
        <ol>
        """
        
        for item in briefItems {
            html += "<li>\(item.text)</li>\n"
        }
        
        html += "</ol>"
        
        if let explain = explain {
            html += "<div class='explain'><strong>说明：</strong> \(explain)</div>"
        }
        
        html += """
        </body>
        </html>
        """
        
        return html
    }
}
