// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements DesignSystem logic in the iOS UI layer.
// - Interfaces:
//   - DesignTokens
//   - NolaPage
//   - NolaPageHeader
//   - NolaCard
//   - NolaDivider
//   - NolaBadge
//   - NolaIconBadge
//   - NolaInlineEmptyState
// - APIs:
//   - none
//
import SwiftUI
import UIKit

// MARK: - Nola Design System

struct DesignTokens {
    // Mobile semantic roles: neutral chrome keeps the conversation in focus.
    private static func mobileColor(_ light: UInt32, _ dark: UInt32) -> Color {
        Color(uiColor: UIColor { traits in
            let rgb = traits.userInterfaceStyle == .dark ? dark : light
            return UIColor(red: CGFloat((rgb >> 16) & 255) / 255,
                           green: CGFloat((rgb >> 8) & 255) / 255,
                           blue: CGFloat(rgb & 255) / 255, alpha: 1)
        })
    }
    static let primaryColor = mobileColor(0x181817, 0xF2F0EB)
    static let actionColor = primaryColor
    static let contentColor = primaryColor
    static let secondaryContentColor = mobileColor(0x5C5B57, 0xA3A09A)
    static let onActionColor = mobileColor(0xFFFFFF, 0x131312)
    static let onAccentColor = onActionColor
    static let accentSubtleColor = mobileColor(0xEAE8E2, 0x292825)
    static let chatBackgroundColor = mobileColor(0xF7F6F3, 0x131312)
    static let successColor = Color("NolaSuccess")
    static let warningColor = Color("NolaWarning")
    static let dangerColor = Color("NolaDanger")
    static let infoColor = mobileColor(0x1F5F8B, 0x8AB8E6)

    // Both Apple clients align fallback surfaces with canonical Warm Ink & Warm Paper tokens.
    static let backgroundColor = mobileColor(0xF7F6F3, 0x131312)
    static let surfaceColor = mobileColor(0xFFFFFF, 0x1B1B19)
    static let elevatedColor = mobileColor(0xF1F0EC, 0x232320)
    static let rowFillColor = mobileColor(0xF1F0EC, 0x232320)
    static let mutedFillColor = Color(.tertiarySystemFill)
    static let borderColor = mobileColor(0xE5E3DD, 0x2E2D2A)
    static let strongBorderColor = Color("NolaControlOutline")
    static let separatorColor = mobileColor(0xE5E3DD, 0x2E2D2A)

    static let overlayStrongOpacity = 0.18
    static let overlayMediumOpacity = 0.12
    static let overlaySoftOpacity = 0.07

    static let spacing_xs: CGFloat = 4
    static let spacing_sm: CGFloat = 8
    static let spacing_md: CGFloat = 12
    static let spacing_lg: CGFloat = 16
    static let spacing_xl: CGFloat = 24

    static let radius_sm: CGFloat = 8
    static let radius_md: CGFloat = 14
    static let radius_lg: CGFloat = 16
    static let radius_xl: CGFloat = 16
    static let radius_pill: CGFloat = 999

    static let rowHeightCompact: CGFloat = 44
    static let rowHeightStandard: CGFloat = 56
    static let iconBadgeSizeSmall: CGFloat = 28
    static let iconBadgeSize: CGFloat = 32
    static let iconBadgeSizeLarge: CGFloat = 40
    static let pageMaxWidth: CGFloat = 840
    static let pageBottomInset: CGFloat = 56

    static let motionFast = 0.10
    static let motionBase = 0.18
    static let motionSlow = 0.22
    static let streamRevealDuration = 0.12
    static let maximumVisualBacklog = 0.32
    static let followIntervalNanoseconds: UInt64 = 16_000_000
    static let followPasses = 12
    static let reducedMotionFollowIntervalNanoseconds: UInt64 = 16_000_000
    static let reducedMotionFollowPasses = 2

    static var disclosureAnimation: Animation {
        .timingCurve(0.20, 0.74, 0.24, 1.00, duration: motionBase)
    }

    static var streamRevealAnimation: Animation {
        .timingCurve(0.16, 0.76, 0.22, 1.00, duration: streamRevealDuration)
    }

    static var responsiveAnimation: Animation {
        .snappy(duration: motionBase, extraBounce: 0)
    }

    static var revealAnimation: Animation {
        .smooth(duration: motionSlow)
    }
}

extension View {
    /// Content stays on its reading axis; reduced motion removes the fade.
    @ViewBuilder
    func nolaDisclosureTransition(reduceMotion: Bool) -> some View {
        if reduceMotion {
            transition(.identity)
        } else {
            transition(.opacity)
        }
    }
}

/// A platform-native disclosure with one motion contract across management
/// pages. Keeping expansion in our design system avoids relying on the
/// OS-version-specific animation used by `DisclosureGroup`.
struct NolaDisclosure<Label: View, Content: View>: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Binding private var isExpanded: Bool
    private let label: Label
    private let content: Content

    init(
        isExpanded: Binding<Bool>,
        @ViewBuilder content: () -> Content,
        @ViewBuilder label: () -> Label
    ) {
        _isExpanded = isExpanded
        self.label = label()
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(reduceMotion ? nil : DesignTokens.disclosureAnimation) {
                    isExpanded.toggle()
                }
            } label: {
                HStack(spacing: DesignTokens.spacing_sm) {
                    label
                    Spacer(minLength: DesignTokens.spacing_sm)
                    Image(systemName: "chevron.down")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .rotationEffect(.degrees(isExpanded ? 180 : 0))
                }
                .frame(minHeight: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityValue(
                isExpanded ? Text("Expanded") : Text("Collapsed")
            )

            if isExpanded {
                content
                    .nolaDisclosureTransition(reduceMotion: reduceMotion)
            }
        }
    }
}

@MainActor
enum NolaHaptics {
    private static let selectionGenerator = UISelectionFeedbackGenerator()
    private static let actionGenerator = UIImpactFeedbackGenerator(style: .light)
    private static let notificationGenerator = UINotificationFeedbackGenerator()

    static func selection() {
        selectionGenerator.selectionChanged()
        selectionGenerator.prepare()
    }

    static func primaryAction() {
        actionGenerator.impactOccurred(intensity: 0.72)
        actionGenerator.prepare()
    }

    static func success() {
        notificationGenerator.notificationOccurred(.success)
        notificationGenerator.prepare()
    }

    static func warning() {
        notificationGenerator.notificationOccurred(.warning)
        notificationGenerator.prepare()
    }
}

struct NolaPressableButtonStyle: ButtonStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .frame(minWidth: 44, minHeight: 44)
            .contentShape(Rectangle())
            .background(configuration.isPressed ? DesignTokens.rowFillColor : Color.clear, in: RoundedRectangle(cornerRadius: DesignTokens.radius_sm, style: .continuous))
            .animation(
                reduceMotion ? nil : .easeOut(duration: DesignTokens.motionFast),
                value: configuration.isPressed
            )
    }
}

struct NolaPageBackground: View {
    var body: some View {
        DesignTokens.backgroundColor
            .ignoresSafeArea()
            .accessibilityHidden(true)
    }
}

struct NolaPage<Content: View>: View {
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: DesignTokens.spacing_xl) {
                content
            }
            .frame(maxWidth: DesignTokens.pageMaxWidth, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .center)
            .padding(.horizontal, horizontalSizeClass == .regular ? DesignTokens.spacing_xl : DesignTokens.spacing_lg)
            .padding(.top, DesignTokens.spacing_lg)
            .padding(.bottom, DesignTokens.pageBottomInset)
        }
        .scrollContentBackground(.hidden)
        .background(NolaPageBackground())
    }
}

struct NolaPageHeader: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let icon: String?
    let title: String
    let subtitle: String?
    var tint: Color = DesignTokens.primaryColor
    var trailing: AnyView?

    init<Trailing: View>(
        icon: String? = nil,
        title: String,
        subtitle: String? = nil,
        tint: Color = DesignTokens.primaryColor,
        @ViewBuilder trailing: () -> Trailing
    ) {
        self.icon = icon
        self.title = title
        self.subtitle = subtitle
        self.tint = tint
        self.trailing = AnyView(trailing())
    }

    init(icon: String? = nil, title: String, subtitle: String? = nil, tint: Color = DesignTokens.primaryColor) {
        self.icon = icon
        self.title = title
        self.subtitle = subtitle
        self.tint = tint
        self.trailing = nil
    }

    var body: some View {
        if dynamicTypeSize.isAccessibilitySize {
            VStack(alignment: .leading, spacing: DesignTokens.spacing_md) {
                heading
                if let trailing { trailing }
            }
        } else if let trailing {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: DesignTokens.spacing_md) {
                    heading.fixedSize(horizontal: true, vertical: false)
                    Spacer(minLength: 0)
                    trailing.fixedSize()
                }
                VStack(alignment: .leading, spacing: DesignTokens.spacing_md) {
                    heading
                    trailing
                }
            }
        } else {
            heading
        }
    }

    private var heading: some View {
        let layout = dynamicTypeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: DesignTokens.spacing_md))
            : AnyLayout(HStackLayout(alignment: .center, spacing: DesignTokens.spacing_md))
        return layout {
            if let icon, !icon.isEmpty {
                ZStack {
                    RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                        .fill(tint.opacity(DesignTokens.overlayMediumOpacity))
                        .frame(width: 42, height: 42)
                    Image(systemName: icon)
                        .font(.system(size: 18, weight: .semibold))
                        .foregroundColor(tint)
                }
            }

            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.title2.weight(.semibold))
                    .foregroundColor(.primary)
                    .fixedSize(horizontal: false, vertical: true)
                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(.subheadline)
                        .foregroundColor(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }

            if !dynamicTypeSize.isAccessibilitySize { Spacer(minLength: DesignTokens.spacing_sm) }

        }
    }
}

struct NolaCard<Content: View>: View {
    let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    private var strokeColor: Color {
        DesignTokens.borderColor
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.spacing_md) {
            content
        }
        .padding(DesignTokens.spacing_lg)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(NolaSurfaceBackground(fill: DesignTokens.surfaceColor, border: strokeColor,
                                          cornerRadius: DesignTokens.radius_md))
    }
}

struct NolaPressableStyle: ButtonStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .frame(minWidth: 44, minHeight: 44)
            .contentShape(Rectangle())
            .background(configuration.isPressed ? DesignTokens.rowFillColor : Color.clear, in: RoundedRectangle(cornerRadius: DesignTokens.radius_sm, style: .continuous))
            .animation(reduceMotion ? nil : DesignTokens.responsiveAnimation, value: configuration.isPressed)
    }
}

struct NolaDivider: View {
    var leading: CGFloat = 0
    var trailing: CGFloat = 0

    var body: some View {
        Rectangle()
            .fill(DesignTokens.separatorColor)
            .frame(height: 1)
            .padding(.leading, leading)
            .padding(.trailing, trailing)
    }
}

struct NolaBadge: View {
    let text: String
    let icon: String?
    let tint: Color

    init(_ text: String, icon: String? = nil, tint: Color = DesignTokens.primaryColor) {
        self.text = text
        self.icon = icon
        self.tint = tint
    }

    var body: some View {
        HStack(spacing: 5) {
            if let icon {
                Image(systemName: icon)
                    .font(.caption2.weight(.bold))
            }
            Text(text)
                .font(.caption.weight(.semibold))
                .lineLimit(1)
        }
        .foregroundColor(tint)
        .padding(.horizontal, 9)
        .padding(.vertical, 5)
        .background(tint.opacity(DesignTokens.overlayMediumOpacity))
        .clipShape(Capsule())
    }
}

struct NolaIconBadge: View {
    let icon: String
    var tint: Color = DesignTokens.primaryColor
    var size: CGFloat = DesignTokens.iconBadgeSize

    var body: some View {
        Image(systemName: icon)
            .font(.system(size: max(12, size * 0.45), weight: .semibold))
            .foregroundColor(tint)
            .frame(width: size, height: size)
            .background(
                Circle()
                    .fill(tint.opacity(DesignTokens.overlayMediumOpacity))
            )
    }
}

struct NolaInlineEmptyState: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let icon: String
    let title: String
    let message: String?
    var tint: Color = DesignTokens.primaryColor
    var actionTitle: String?
    var action: (() -> Void)?

    init(
        icon: String,
        title: String,
        message: String? = nil,
        tint: Color = DesignTokens.primaryColor,
        actionTitle: String? = nil,
        action: (() -> Void)? = nil
    ) {
        self.icon = icon
        self.title = title
        self.message = message
        self.tint = tint
        self.actionTitle = actionTitle
        self.action = action
    }

    var body: some View {
        let layout = dynamicTypeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: DesignTokens.spacing_md))
            : AnyLayout(HStackLayout(alignment: .top, spacing: DesignTokens.spacing_md))
        return layout {
            NolaIconBadge(icon: icon, tint: tint, size: DesignTokens.iconBadgeSizeLarge)

            VStack(alignment: .leading, spacing: 5) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.primary)
                    .fixedSize(horizontal: false, vertical: true)
                if let message, !message.isEmpty {
                    Text(message)
                        .font(.subheadline)
                        .foregroundColor(.secondary)
                        .multilineTextAlignment(.leading)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let actionTitle, let action {
                    Button(action: action) {
                        Text(actionTitle)
                            .font(.subheadline.weight(.semibold))
                            .frame(minHeight: DesignTokens.rowHeightCompact)
                            .contentShape(Rectangle())
                    }
                        .buttonStyle(.bordered)
                        .padding(.top, 2)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(DesignTokens.spacing_md)
        .frame(maxWidth: .infinity, minHeight: DesignTokens.rowHeightStandard, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .fill(DesignTokens.rowFillColor)
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
    }
}

struct NolaStatCard: View {
    let title: String
    let value: String
    let icon: String
    let tint: Color
    var subtitle: String?

    var body: some View {
        NolaCard {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(title)
                        .font(.caption.weight(.semibold))
                        .foregroundColor(.secondary)
                    Text(value)
                        .font(.title2.weight(.bold))
                    if let subtitle {
                        Text(subtitle)
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                }
                Spacer()
                Image(systemName: icon)
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundColor(tint)
                    .frame(width: 38, height: 38)
                    .background(tint.opacity(DesignTokens.overlayMediumOpacity))
                    .clipShape(RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous))
            }
        }
    }
}

struct NolaEmptyState: View {
    let icon: String
    let title: String
    let message: String
    var actionTitle: String?
    var action: (() -> Void)?

    var body: some View {
        NolaCard {
            VStack(alignment: .leading, spacing: DesignTokens.spacing_md) {
                Image(systemName: icon)
                    .font(.system(size: 24, weight: .semibold))
                    .foregroundColor(DesignTokens.primaryColor)
                    .frame(width: 52, height: 52)
                    .background(DesignTokens.primaryColor.opacity(DesignTokens.overlayMediumOpacity))
                    .clipShape(RoundedRectangle(cornerRadius: DesignTokens.radius_xl, style: .continuous))
                Text(title)
                    .font(.headline)
                Text(message)
                    .font(.subheadline)
                    .foregroundColor(.secondary)
                    .multilineTextAlignment(.leading)
                if let actionTitle, let action {
                    Button(actionTitle, action: action)
                        .buttonStyle(.borderedProminent)
                        .tint(DesignTokens.primaryColor)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

struct NolaSectionTitle: View {
    let icon: String?
    let title: String

    init(icon: String? = nil, title: String) {
        self.icon = icon
        self.title = title
    }

    var body: some View {
        HStack(spacing: 8) {
            if let icon, !icon.isEmpty {
                Image(systemName: icon)
                    .font(.caption.weight(.bold))
                    .foregroundColor(DesignTokens.primaryColor)
                    .frame(width: 14)
            }
            Text(title.uppercased())
                .font(.caption.weight(.bold))
                .foregroundColor(.secondary)
                .tracking(0.8)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// Minimum interaction bounds live inside the button label, not outside its action.
struct NolaActionButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .frame(minWidth: 44, minHeight: 44)
            .contentShape(Rectangle())
            .background(NolaSurfaceBackground(fill: configuration.isPressed ? DesignTokens.rowFillColor : DesignTokens.surfaceColor,
                                              border: DesignTokens.borderColor,
                                              cornerRadius: DesignTokens.radius_sm, raised: false))
            .opacity(isEnabled ? (configuration.isPressed ? 0.78 : 1) : 0.45)
    }
}

/// The title, chevron and trailing whitespace are one accessible disclosure action.
struct NolaRowDisclosureStyle: DisclosureGroupStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Button {
                withAnimation(reduceMotion ? nil : DesignTokens.disclosureAnimation) {
                    configuration.isExpanded.toggle()
                }
            } label: {
                HStack(spacing: 8) {
                    configuration.label
                    Spacer(minLength: 8)
                    Image(systemName: "chevron.down")
                        .font(.caption.weight(.semibold))
                        .rotationEffect(.degrees(configuration.isExpanded ? 180 : 0))
                        .accessibilityHidden(true)
                }
                .padding(.horizontal, DesignTokens.spacing_sm)
                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(NolaActionButtonStyle())
            .accessibilityValue(configuration.isExpanded ? String(localized: "Expanded") : String(localized: "Collapsed"))
            if configuration.isExpanded {
                configuration.content.nolaDisclosureTransition(reduceMotion: reduceMotion)
            }
        }
    }
}
