// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelManagementView logic in the iOS UI layer.
// - Interfaces:
//   - ModelManagementViewNew
// - APIs:
//   - none
//
import SwiftUI

struct ModelManagementViewNew: View {
    @EnvironmentObject var container: ServiceContainer

    var body: some View {
        ModelsView()
            .environmentObject(container)
    }
}

#Preview {
    ModelManagementViewNew()
        .environmentObject(ServiceContainer())
}
