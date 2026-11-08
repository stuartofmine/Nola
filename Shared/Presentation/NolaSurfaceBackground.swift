// SPDX-License-Identifier: Apache-2.0
import SwiftUI

/// One opaque surface treatment for both Apple clients. The highlight belongs
/// to the surface, so text and icons never inherit a shadow or a gradient.
struct NolaSurfaceBackground: View {
    var fill: Color
    var border: Color
    var cornerRadius: CGFloat
    var raised = true
    @Environment(\.colorScheme) private var scheme
    @Environment(\.colorSchemeContrast) private var contrast

    private var shape: RoundedRectangle {
        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
    }

    var body: some View {
        shape.fill(fill)
            .overlay {
                shape.fill(LinearGradient(
                    colors: [.white.opacity(scheme == .dark ? 0.045 : 0.34),
                             .black.opacity(scheme == .dark ? 0.025 : 0.018)],
                    startPoint: .topLeading, endPoint: .bottomTrailing
                ))
            }
            .overlay {
                shape.strokeBorder(contrast == .increased ? Color.primary.opacity(0.5) : border,
                                   lineWidth: contrast == .increased ? 1.25 : 0.75)
            }
            .overlay {
                shape.inset(by: 1.25).strokeBorder(
                    LinearGradient(colors: [.white.opacity(scheme == .dark ? 0.09 : 0.65), .clear],
                                   startPoint: .top, endPoint: .center), lineWidth: 0.75
                )
            }
            .shadow(color: .black.opacity(scheme == .dark ? 0.19 : 0.055),
                    radius: raised ? 10 : 3, x: 0, y: raised ? 3 : 1)
            .accessibilityHidden(true)
    }
}
