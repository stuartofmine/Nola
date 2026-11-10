// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ContentView logic in the iOS app.
// - Interfaces:
//   - ContentView
// - APIs:
//   - none
//
//
//  ContentView.swift
//  Nola
//
//

import SwiftUI
import CoreData

// ContentView is now a wrapper that displays the main tab navigation or onboarding
struct ContentView: View {
    @EnvironmentObject private var container: ServiceContainer
    @AppStorage("hasCompletedOnboarding") private var hasCompletedOnboarding = false
    
    var body: some View {
        if hasCompletedOnboarding {
            MainTabView()
        } else {
            OnboardingView()
        }
    }
}
