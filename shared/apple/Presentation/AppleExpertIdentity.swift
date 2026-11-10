// SPDX-License-Identifier: Apache-2.0
import SwiftUI

/// Decorative identity beside the message; the complete accessible name is
/// rendered once by AppleExpertName in the external identity column.
struct AppleExpertAvatar: View {
    let name: String
    @ScaledMetric(relativeTo: .caption) private var diameter: CGFloat = 28

    init(name: String, size: CGFloat = 28) {
        self.name = name
        _diameter = ScaledMetric(wrappedValue: size, relativeTo: .caption)
    }

    var body: some View {
        Text(String(name.prefix(1)))
            .font(.system(size: max(12, diameter * 0.46), weight: .semibold))
            .foregroundStyle(.primary)
            .frame(width: diameter, height: diameter)
            .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 11, style: .continuous))
            .accessibilityHidden(true)
    }
}

struct AppleExpertName: View {
    let name: String
    var alignment: HorizontalAlignment = .leading
    @ScaledMetric(relativeTo: .caption) private var textSize: CGFloat = 12

    var body: some View {
        Text(name)
            .font(.system(size: max(12, textSize), weight: .semibold))
            .foregroundStyle(.primary)
            .multilineTextAlignment(alignment == .center ? .center : .leading)
            .lineLimit(nil)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: alignment == .center ? .center : .leading)
    }
}
