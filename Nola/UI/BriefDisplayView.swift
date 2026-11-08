// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements BriefDisplayView logic in the iOS UI layer.
// - Interfaces:
//   - BriefDisplayView
// - APIs:
//   - none
//
//
//  BriefDisplayView.swift
//  Nola
//
//  Simple view for displaying brief content

import SwiftUI

struct BriefDisplayView: View {
    let title: String
    let items: [String]
    let explanation: String?
    let briefType: String // "morning" or "evening"
    
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            // Header
            HStack {
                Image(systemName: briefType == "morning" ? "sunrise.fill" : "sunset.fill")
                    .font(.title2)
                    .foregroundColor(briefType == "morning" ? .orange : .purple)
                
                Text(title)
                    .font(.headline)
                
                Spacer()
            }
            
            Divider()
            
            // Items List
            VStack(alignment: .leading, spacing: 12) {
                ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                    HStack(alignment: .top, spacing: 12) {
                        Circle()
                            .fill(Color.blue)
                            .frame(width: 6, height: 6)
                            .padding(.top, 6)
                        
                        Text(item)
                            .font(.body)
                        
                        Spacer()
                    }
                }
            }
            
            // Explanation
            if let explanation = explanation {
                VStack(alignment: .leading, spacing: 8) {
                    Divider()
                    
                    HStack(spacing: 8) {
                        Image(systemName: "lightbulb.fill")
                            .foregroundColor(.yellow)
                        
                        Text(L10n.modelTestPromptLabel)
                            .font(.caption)
                            .fontWeight(.semibold)
                    }
                    
                    Text(explanation)
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .lineLimit(3)
                }
            }
            
            Spacer()
        }
        .padding()
        .background(Color(red: 0.95, green: 0.95, blue: 0.95))
        .cornerRadius(12)
    }
}

#Preview {
    BriefDisplayView(
        title: NSLocalizedString("autogen.BriefDisplayView.15081b04", comment: "AUTOGEN: localized text"),
        items: [
            NSLocalizedString("autogen.BriefDisplayView.7790cd26", comment: "AUTOGEN: localized text"),
            NSLocalizedString("autogen.BriefDisplayView.20115727", comment: "AUTOGEN: localized text"),
            NSLocalizedString("autogen.BriefDisplayView.da6d8802", comment: "AUTOGEN: localized text")
        ],
        explanation: NSLocalizedString("autogen.BriefDisplayView.ad6f4e5c", comment: "AUTOGEN: localized text"),
        briefType: "morning"
    )
}
