// SPDX-License-Identifier: Apache-2.0
import SwiftUI

/// Opaque semantic surfaces. Ordinary content has no bevel or elevation;
/// floating surfaces can explicitly opt into one restrained shadow.
struct NolaSurfaceBackground: View {
    var fill: Color
    var border: Color
    var cornerRadius: CGFloat
    var raised = false
    @Environment(\.colorSchemeContrast) private var contrast

    private var shape: RoundedRectangle {
        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
    }

    private var surface: some View {
        shape.fill(fill)
            .overlay {
                shape.strokeBorder(contrast == .increased ? Color.primary.opacity(0.5) : border,
                                   lineWidth: contrast == .increased ? 1.25 : 0.75)
            }
    }

    var body: some View {
        Group {
            if raised {
                surface.shadow(color: .black.opacity(0.06), radius: 8, x: 0, y: 2)
            } else {
                surface
            }
        }.accessibilityHidden(true)
    }
}
