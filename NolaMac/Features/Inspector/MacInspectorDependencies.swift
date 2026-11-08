// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Declares the services supplied by the macOS composition root to the conversation inspector.

@MainActor
struct MacInspectorDependencies {
    let dataRegistry: MacDataSourceRegistry
    let platformData: MacPlatformDataService
    let webSources: MacWebSourceStore
    let chromeMessaging: any MacInspectorChromeMessaging
}
