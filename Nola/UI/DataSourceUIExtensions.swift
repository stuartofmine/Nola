// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements DataSourceUIExtensions logic in the iOS UI layer.
// - Interfaces:
//   - DataSourceType
//   - AuthorizationStatus
// - APIs:
//   - none
//
//
//  DataSourceUIExtensions.swift
//  Nola
//
//  SwiftUI-only helpers for shared data source types.
//

import SwiftUI

extension DataSourceType {
    var color: Color {
        switch self {
        case .contacts: return .blue
        case .calendars: return .orange
        case .reminders: return .green
        case .health: return .red
        case .url: return .purple
        }
    }
}

extension AuthorizationStatus {
    var uiColor: Color {
        switch self {
        case .notDetermined: return .gray
        case .authorized: return .green
        case .insufficient: return .orange
        case .denied: return .red
        case .restricted: return .orange
        case .unavailable: return .gray
        }
    }

    var systemImage: String {
        switch self {
        case .notDetermined: return "questionmark.circle.fill"
        case .authorized: return "checkmark.circle.fill"
        case .insufficient: return "exclamationmark.triangle.fill"
        case .denied: return "xmark.circle.fill"
        case .restricted: return "exclamationmark.triangle.fill"
        case .unavailable: return "exclamationmark.triangle.fill"
        }
    }
}
