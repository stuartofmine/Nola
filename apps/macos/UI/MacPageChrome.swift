// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides shared, restrained chrome for top-level macOS feature pages.

import AppKit
import SwiftUI

/// A Settings scene may retain its hosting view after its native window closes.
struct MacWindowCloseObserver: NSViewRepresentable {
    let onClose: () -> Void

    func makeNSView(context: Context) -> ObserverView { ObserverView() }
    func updateNSView(_ view: ObserverView, context: Context) { view.onClose = onClose }

    final class ObserverView: NSView {
        var onClose: (() -> Void)?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            let center = NotificationCenter.default
            center.removeObserver(self, name: NSWindow.willCloseNotification, object: nil)
            if let window {
                center.addObserver(self, selector: #selector(windowClosing(_:)), name: NSWindow.willCloseNotification, object: window)
            }
        }

        @objc private func windowClosing(_ notification: Notification) { onClose?() }
    }
}

/// Protects a standalone editor's native close button and Command-W as well as
/// its in-content Cancel action. Forward every unrelated delegate method to
/// SwiftUI's delegate rather than taking over window lifecycle ownership.
struct MacEditorWindowCloseGuard: NSViewRepresentable {
    let shouldPreventClose: () -> Bool
    let onCloseAttempt: () -> Void
    var onCloseAttemptWithWindow: ((NSWindow) -> Void)? = nil

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> WindowProbe {
        let probe = WindowProbe()
        probe.onWindowChanged = { [weak coordinator = context.coordinator] window in
            coordinator?.attach(to: window)
        }
        return probe
    }

    func updateNSView(_ view: WindowProbe, context: Context) {
        context.coordinator.shouldPreventClose = shouldPreventClose
        context.coordinator.onCloseAttempt = onCloseAttempt
        context.coordinator.onCloseAttemptWithWindow = onCloseAttemptWithWindow
        context.coordinator.attach(to: view.window)
    }

    static func dismantleNSView(_ view: WindowProbe, coordinator: Coordinator) {
        view.onWindowChanged = nil
        coordinator.detach()
    }

    final class WindowProbe: NSView {
        var onWindowChanged: ((NSWindow?) -> Void)?
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            onWindowChanged?(window)
        }
    }

    @MainActor
    final class Coordinator: NSObject, NSWindowDelegate {
        var shouldPreventClose: () -> Bool = { false }
        var onCloseAttempt: () -> Void = {}
        var onCloseAttemptWithWindow: ((NSWindow) -> Void)?
        private weak var window: NSWindow?
        private weak var forwardedDelegate: (any NSWindowDelegate)?
        private var previousEditedState = false

        func attach(to window: NSWindow?) {
            guard let window else { detach(); return }
            if self.window !== window {
                detach()
                self.window = window
                previousEditedState = window.isDocumentEdited
            }
            if window.delegate !== self {
                forwardedDelegate = window.delegate
                window.delegate = self
            }
            window.isDocumentEdited = shouldPreventClose()
        }

        func detach() {
            if let window, window.delegate === self {
                window.delegate = forwardedDelegate
                window.isDocumentEdited = previousEditedState
            }
            window = nil
            forwardedDelegate = nil
        }

        func windowShouldClose(_ sender: NSWindow) -> Bool {
            if shouldPreventClose() {
                if let onCloseAttemptWithWindow { onCloseAttemptWithWindow(sender) }
                else { onCloseAttempt() }
                return false
            }
            return forwardedDelegate?.windowShouldClose?(sender) ?? true
        }

        nonisolated override func responds(to selector: Selector!) -> Bool {
            if super.responds(to: selector) { return true }
            return MainActor.assumeIsolated { forwardedDelegate?.responds(to: selector) ?? false }
        }

        nonisolated override func forwardingTarget(for selector: Selector!) -> Any? {
            // This Objective-C hook runs synchronously. Check main-actor
            // isolation while taking a strong local reference; return Void
            // from assumeIsolated, not a non-Sendable delegate across actors.
            var target: AnyObject?
            MainActor.assumeIsolated { target = forwardedDelegate }
            return target
        }
    }
}

enum MacDesignTokens {
    /// Canonical Monochrome Obsidian & Porcelain RGB fallbacks synchronized with shared/design-tokens/tokens.json.
    /// Native macOS chrome continues binding semantic Asset Catalog entries through `MacDesignColor`.
    enum Palette {
        enum Light {
            static let canvas: UInt32 = 0xFAFAFA
            static let surface: UInt32 = 0xFFFFFF
            static let surfaceSubtle: UInt32 = 0xF2F2F4
            static let surfaceElevated: UInt32 = 0xFFFFFF
            static let ink: UInt32 = 0x111113
            static let inkSecondary: UInt32 = 0x52525B
            static let inkMuted: UInt32 = 0x63636E
            static let border: UInt32 = 0xE4E4E7
            static let outline: UInt32 = 0x71717A
            static let accent: UInt32 = 0x18181B
            static let onAccent: UInt32 = 0xFFFFFF
            static let accentSubtle: UInt32 = 0xEBEBEF
            static let success: UInt32 = 0x147537
            static let warning: UInt32 = 0x925605
            static let danger: UInt32 = 0xC81E1E
            static let info: UInt32 = 0x2563EB
        }

        enum Dark {
            static let canvas: UInt32 = 0x0b0b0c
            static let surface: UInt32 = 0x131315
            static let surfaceSubtle: UInt32 = 0x1a1a1d
            static let surfaceElevated: UInt32 = 0x242428
            static let ink: UInt32 = 0xF5F5F7
            static let inkSecondary: UInt32 = 0xA1A1AA
            static let inkMuted: UInt32 = 0x8E8E93
            static let border: UInt32 = 0x26262a
            static let outline: UInt32 = 0x7a7a82
            static let accent: UInt32 = 0xF5F5F7
            static let onAccent: UInt32 = 0x0B0B0C
            static let accentSubtle: UInt32 = 0x202024
            static let success: UInt32 = 0x5CD68D
            static let warning: UInt32 = 0xF0B85A
            static let danger: UInt32 = 0xF28080
            static let info: UInt32 = 0x78B4F8
        }

        static func fallbackNSColor(light: UInt32, dark: UInt32) -> NSColor {
            NSColor(name: nil) { appearance in
                let match = appearance.bestMatch(from: [.darkAqua, .vibrantDark, .aqua, .vibrantLight])
                let rgb = (match == .darkAqua || match == .vibrantDark) ? dark : light
                return NSColor(
                    srgbRed: CGFloat((rgb >> 16) & 0xFF) / 255.0,
                    green: CGFloat((rgb >> 8) & 0xFF) / 255.0,
                    blue: CGFloat(rgb & 0xFF) / 255.0,
                    alpha: 1.0
                )
            }
        }

        static func fallbackColor(light: UInt32, dark: UInt32) -> Color {
            Color(nsColor: fallbackNSColor(light: light, dark: dark))
        }
    }

    enum Typography {
        static let inspectorBody: CGFloat = 13
        static let inspectorDetail: CGFloat = 12
        static let inspectorCaption: CGFloat = 11
    }
    enum Spacing {
        static let xxs: CGFloat = 4
        static let xs: CGFloat = 6
        static let sm: CGFloat = 8
        static let md: CGFloat = 12
        static let lg: CGFloat = 16
        static let xl: CGFloat = 20
        static let xxl: CGFloat = 24
        static let xxxl: CGFloat = 32
    }

    enum Radius {
        static let control: CGFloat = 10
        static let field: CGFloat = 10
        static let panel: CGFloat = 14
        static let floating: CGFloat = 20
        static let message: CGFloat = 18
    }

    enum Control {
        static let iconButton: CGFloat = 36
        static let textInsets = EdgeInsets(top: 6, leading: 10, bottom: 6, trailing: 10)
        static let compactRow: CGFloat = 36
        static let featureIcon: CGFloat = 40
        static let pageHeader: CGFloat = 56
    }

    enum Layout {
        static let sidebarWidth: CGFloat = 244
        static let inspectorWidth: CGFloat = 360
        static let contentWidth: CGFloat = 900
        static let pageGutter: CGFloat = 24
        // Give desktop conversations more room without turning prose into
        // full-window lines. The composer and transcript grow together;
        // compact windows still use their available width.
        static let chatTranscriptMaxWidth: CGFloat = 720
        static let chatProseMaxWidth: CGFloat = 720
        static let chatTranscriptHorizontalPadding: CGFloat = 24
        static let titlebarClearance: CGFloat = 30
    }

    enum Motion {
        static let hover = 0.10
        static let selection = 0.16
        static let disclosure = 0.18
        static let structural = 0.22
        static let contentReveal = 0.12
        static let maximumVisualBacklog = 0.32
        static let statusPulse = 1.60

        static var responsive: Animation {
            .timingCurve(0.20, 0.72, 0.24, 1.00, duration: selection)
        }

        static var structure: Animation {
            .timingCurve(0.18, 0.80, 0.22, 1.00, duration: structural)
        }

        static var reveal: Animation {
            .timingCurve(0.16, 0.76, 0.22, 1.00, duration: contentReveal)
        }

        static var disclosureAnimation: Animation {
            .timingCurve(0.20, 0.74, 0.24, 1.00, duration: disclosure)
        }

        static var pageChange: Animation {
            .timingCurve(0.20, 0.76, 0.24, 1.00, duration: disclosure)
        }
    }
}

extension View {
    /// A shared, restrained transition for content that is revealed in place.
    /// The parent animates height; fade the contents without scaling text glyphs.
    @ViewBuilder
    func macDisclosureTransition(reduceMotion: Bool) -> some View {
        if reduceMotion {
            transition(.identity)
        } else {
            transition(.opacity)
        }
    }
}

enum MacChatBubbleLayout {
    static func transcriptWidth(availableWidth: CGFloat) -> CGFloat {
        let safeWidth = max(0, availableWidth - (MacDesignTokens.Layout.chatTranscriptHorizontalPadding * 2))
        guard safeWidth > 0 else { return 0 }

        // One continuous rule: breakpoint percentages used to make the column
        // shrink while the window grew. Keep compact windows edge-to-edge and
        // bound wide-screen prose, with the same column used by the composer.
        return min(safeWidth, MacDesignTokens.Layout.chatTranscriptMaxWidth)
    }

    static func userBubbleWidth(transcriptWidth: CGFloat) -> CGFloat {
        let widthRatio: CGFloat
        switch transcriptWidth {
        case ..<720:
            widthRatio = 0.88
        case ..<1_400:
            widthRatio = 0.76
        default:
            widthRatio = 0.68
        }
        return min(max(0, transcriptWidth), min(620, max(260, transcriptWidth * widthRatio)))
    }
}

enum MacDesignColor {
    static let onAccent = Color("NolaOnAccent")
    static let primaryAction = Color("AccentColor")
    static let onPrimaryAction = Color("NolaOnAccent")
    static let surfaceRaised = Color("NolaSurfaceRaised")
    static let controlOutline = Color("NolaControlOutline")
    static let brand = Color("AccentColor")
    static let canvas = Color("NolaCanvas")
    static let chrome = MacDesignTokens.Palette.fallbackColor(
        light: MacDesignTokens.Palette.Light.surfaceSubtle,
        dark: MacDesignTokens.Palette.Dark.surface
    )
    static let sidebar = MacDesignTokens.Palette.fallbackColor(
        light: MacDesignTokens.Palette.Light.surfaceSubtle,
        dark: MacDesignTokens.Palette.Dark.surface
    )
    static let inspector = MacDesignTokens.Palette.fallbackColor(
        light: MacDesignTokens.Palette.Light.surfaceSubtle,
        dark: MacDesignTokens.Palette.Dark.surface
    )
    static let subtleFill = MacDesignTokens.Palette.fallbackColor(
        light: MacDesignTokens.Palette.Light.surfaceSubtle,
        dark: MacDesignTokens.Palette.Dark.surfaceSubtle
    )
    static let sidebarGlass = MacDesignTokens.Palette.fallbackColor(
        light: MacDesignTokens.Palette.Light.surfaceSubtle,
        dark: MacDesignTokens.Palette.Dark.surface
    ).opacity(0.68)
    static let inspectorGlass = MacDesignTokens.Palette.fallbackColor(
        light: MacDesignTokens.Palette.Light.surfaceSubtle,
        dark: MacDesignTokens.Palette.Dark.surface
    ).opacity(0.72)
    static let panel = Color("NolaSurface")
    static let hover = Color("NolaControlHover")
    static let pressed = Color("NolaControlPressed")
    static let selected = Color("NolaAccentSubtle")
    static let borderSubtle = Color("NolaDivider")
    static let success = Color("NolaSuccess")
    static let warning = Color("NolaWarning")
    static let danger = Color("NolaDanger")
    static let info = MacDesignTokens.Palette.fallbackColor(
        light: MacDesignTokens.Palette.Light.info,
        dark: MacDesignTokens.Palette.Dark.info
    )
    static let switchTrackOn = MacDesignTokens.Palette.fallbackColor(light: 0x147537, dark: 0x5CD68D)
    static let switchTrackOnHover = MacDesignTokens.Palette.fallbackColor(light: 0x1D5E3B, dark: 0x4CC67D)
    static let switchTrackOff = MacDesignTokens.Palette.fallbackColor(light: 0xE4E4E7, dark: 0x2E2E33)
    static let switchTrackOffHover = MacDesignTokens.Palette.fallbackColor(light: 0xD4D4D8, dark: 0x3A3A40)
    static let switchBorderOff = MacDesignTokens.Palette.fallbackColor(light: 0xC4C4CA, dark: 0x48484F)
}

private struct MacGlobalWorkspaceChromeKey: EnvironmentKey {
    static let defaultValue = false
}
extension EnvironmentValues {
    var macUsesGlobalWorkspaceChrome: Bool {
        get { self[MacGlobalWorkspaceChromeKey.self] }
        set { self[MacGlobalWorkspaceChromeKey.self] = newValue }
    }
}

enum MacPageNavigationKind {
    case sidebar, back

    var symbol: String { self == .back ? "chevron.left" : "sidebar.left" }
}

struct MacPageHeader<Subtitle: View, Actions: View>: View {
    let title: LocalizedStringKey
    let symbol: String
    let navigationAction: (() -> Void)?
    let navigationKind: MacPageNavigationKind
    let navigationLabel: LocalizedStringKey
    @Environment(\.macUsesGlobalWorkspaceChrome) private var usesGlobalWorkspaceChrome
    @ViewBuilder let subtitle: () -> Subtitle
    @ViewBuilder let actions: () -> Actions

    init(
        _ title: LocalizedStringKey,
        symbol: String,
        navigationAction: (() -> Void)? = nil,
        navigationKind: MacPageNavigationKind = .sidebar,
        navigationLabel: LocalizedStringKey = "Toggle Sidebar",
        @ViewBuilder subtitle: @escaping () -> Subtitle,
        @ViewBuilder actions: @escaping () -> Actions
    ) {
        self.title = title
        self.symbol = symbol
        self.navigationAction = navigationAction
        self.navigationKind = navigationKind
        self.navigationLabel = navigationLabel
        self.subtitle = subtitle
        self.actions = actions
    }

    var body: some View {
        Group {
            if usesGlobalWorkspaceChrome {
                HStack(alignment: .top, spacing: 16) {
                    if navigationKind == .back { navigationButton }
                    VStack(alignment: .leading, spacing: 6) {
                        Text(title).font(.system(size: 20, weight: .semibold))
                        subtitle().font(.callout).foregroundStyle(.secondary)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                    actionGroup
                }
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.top, 24).padding(.bottom, 12)
            } else {
                ViewThatFits(in: .horizontal) {
                    wideHeader
                    compactHeader
                }
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.vertical, MacDesignTokens.Spacing.sm)
                .frame(minHeight: MacDesignTokens.Control.pageHeader)
                .background(MacDesignColor.chrome)
                .overlay(alignment: .bottom) {
                    Rectangle()
                        .fill(MacDesignColor.borderSubtle)
                        .frame(height: 1)
                }
            }
        }
    }

    private var wideHeader: some View {
        HStack(spacing: MacDesignTokens.Spacing.sm) {
            navigationButton
            pageSymbol

            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.system(size: 15, weight: .semibold))
                    .lineLimit(1)
                subtitle()
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .layoutPriority(1)

            Spacer(minLength: 20)

            actionGroup
        }
    }

    private var compactHeader: some View {
        HStack(spacing: MacDesignTokens.Spacing.sm) {
            navigationButton
            pageSymbol
            Text(title)
                .font(.system(size: 15, weight: .semibold))
                .lineLimit(1)
                .layoutPriority(1)
            Spacer(minLength: MacDesignTokens.Spacing.sm)
            actionGroup
                .labelStyle(.iconOnly)
        }
    }

    @ViewBuilder
    private var navigationButton: some View {
        if let navigationAction {
            Button(action: navigationAction) {
                Image(systemName: navigationKind.symbol)
                    .font(.system(size: 12, weight: .semibold))
                    .frame(
                        width: MacDesignTokens.Control.iconButton,
                        height: MacDesignTokens.Control.iconButton
                    )
            }
            .macToolbarIcon()
            .help(navigationLabel)
            .accessibilityLabel(navigationLabel)
            .accessibilityIdentifier(navigationKind == .back ? "page.navigation.back" : "page.navigation.sidebar")
        }
    }

    private var pageSymbol: some View {
        Image(systemName: symbol)
            .font(.system(size: 14, weight: .medium, design: .rounded))
            .foregroundStyle(MacDesignColor.brand.opacity(0.88))
            .frame(width: 22, height: 22)
            .accessibilityHidden(true)
    }

    private var actionGroup: some View {
        HStack(spacing: MacDesignTokens.Spacing.sm) {
            actions()
        }
        .controlSize(.small)
        .fixedSize(horizontal: true, vertical: false)
    }
}

struct MacPageBackground: View {
    var body: some View {
        MacDesignColor.canvas
            .ignoresSafeArea()
    }
}

private struct MacPanelModifier: ViewModifier {
    let cornerRadius: CGFloat

    func body(content: Content) -> some View {
        content
            .background(NolaSurfaceBackground(fill: MacDesignColor.panel,
                                              border: MacDesignColor.borderSubtle,
                                              cornerRadius: cornerRadius))
    }
}

private struct MacToolbarPressButtonStyle: ButtonStyle {
    var isActive = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .frame(minWidth: MacDesignTokens.Control.iconButton, minHeight: MacDesignTokens.Control.iconButton)
            .contentShape(Rectangle())
            .background(configuration.isPressed && !isActive ? MacDesignColor.pressed : Color.clear, in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control))
            .animation(
                reduceMotion ? nil : MacDesignTokens.Motion.responsive,
                value: configuration.isPressed
            )
    }
}

private struct MacToolbarIconModifier: ViewModifier {
    let isActive: Bool
    var fixedWidth = true
    @State private var isHovered = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        content
            .buttonStyle(MacToolbarPressButtonStyle(isActive: isActive))
            .frame(
                width: fixedWidth ? MacDesignTokens.Control.iconButton : nil,
                height: MacDesignTokens.Control.iconButton
            )
            .foregroundStyle(isActive ? Color.primary : Color.secondary)
            .background(
                backgroundColor,
                in: RoundedRectangle(
                    cornerRadius: MacDesignTokens.Radius.control,
                    style: .continuous
                )
            )
            .macSelectionHighlight(isSelected: isActive)
            .contentShape(Rectangle())
            .opacity(isEnabled ? 1 : 0.45)
            .onHover { hovering in
                withAnimation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover)) {
                    isHovered = isEnabled && hovering
                }
            }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovered = false } }
    }

    private var backgroundColor: Color {
        return !isActive && isEnabled && isHovered ? MacDesignColor.hover : .clear
    }
}

private struct MacSelectionHighlightModifier: ViewModifier {
    let isSelected: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorSchemeContrast) private var contrast

    func body(content: Content) -> some View {
        content
            .background(
                isSelected ? (contrast == .increased ? Color.primary.opacity(0.14) : MacDesignColor.selected) : .clear,
                in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
            )
            .accessibilityAddTraits(isSelected ? .isSelected : [])
            .animation(reduceMotion ? nil : MacDesignTokens.Motion.responsive, value: isSelected)
    }
}

private struct MacSelectableListRowModifier: ViewModifier {
    let isSelected: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        content
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, MacDesignTokens.Spacing.sm)
            .macSelectionHighlight(isSelected: isSelected)
            .contentShape(Rectangle())
            .listRowInsets(EdgeInsets(top: 2, leading: 4, bottom: 2, trailing: 4))
            .listRowBackground(Color.clear)
            .animation(
                reduceMotion ? nil : MacDesignTokens.Motion.responsive,
                value: isSelected
            )
    }
}

extension View {
    func macSelectionHighlight(isSelected: Bool) -> some View {
        modifier(MacSelectionHighlightModifier(isSelected: isSelected))
    }

    func macPanel(cornerRadius: CGFloat = MacDesignTokens.Radius.panel) -> some View {
        modifier(MacPanelModifier(cornerRadius: cornerRadius))
    }

    func macToolbarIcon(isActive: Bool = false, fixedWidth: Bool = true) -> some View {
        modifier(MacToolbarIconModifier(isActive: isActive, fixedWidth: fixedWidth))
    }

    func macSelectableListRow(isSelected: Bool) -> some View {
        modifier(MacSelectableListRowModifier(isSelected: isSelected))
    }

    func macComposerPill(isActive: Bool = false, tint: Color = MacDesignColor.brand) -> some View {
        modifier(MacComposerPillModifier(isActive: isActive, tint: tint))
    }
}

struct MacPillMenuStyle: MenuStyle {
    var height: CGFloat = 28
    var isActive = false
    var tint: Color = MacDesignColor.brand

    func makeBody(configuration: Configuration) -> some View {
        Menu(configuration)
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .padding(.leading, 8)
            .padding(.trailing, 20)
            .frame(minHeight: height)
            .overlay(alignment: .trailing) {
                Image(systemName: "chevron.down")
                    .font(.system(size: 8.5, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .padding(.trailing, 8)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
            }
            .macComposerPill(isActive: isActive, tint: tint)
            .overlay {
                MacMenuHitExpandOverlay()
            }
    }
}

struct MacToolbarMenuStyle: MenuStyle {
    var size: CGFloat = MacDesignTokens.Control.iconButton
    var cornerRadius: CGFloat = MacDesignTokens.Radius.control

    func makeBody(configuration: Configuration) -> some View {
        MacToolbarMenuBody(configuration: configuration, size: size, cornerRadius: cornerRadius)
    }
}

private struct MacToolbarMenuBody: View {
    let configuration: MenuStyleConfiguration
    let size: CGFloat
    let cornerRadius: CGFloat
    @State private var isHovered = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Menu(configuration)
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .labelStyle(.iconOnly)
            .frame(width: size, height: size)
            .foregroundStyle(Color.secondary)
            .background(
                isEnabled && isHovered ? MacDesignColor.hover : Color.clear,
                in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
            )
            .contentShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .opacity(isEnabled ? 1 : 0.45)
            .onHover { isHovered = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovered = false } }
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovered)
            .overlay {
                MacMenuHitExpandOverlay()
            }
    }
}

struct MacInlineIconButtonStyle: ButtonStyle {
    var size: CGFloat = 24
    var cornerRadius: CGFloat = 6

    func makeBody(configuration: Configuration) -> some View {
        MacInlineIconButtonBody(
            label: configuration.label,
            isPressed: configuration.isPressed,
            size: size,
            cornerRadius: cornerRadius
        )
    }
}

private struct MacInlineIconButtonBody<Label: View>: View {
    let label: Label
    let isPressed: Bool
    let size: CGFloat
    let cornerRadius: CGFloat
    @State private var isHovered = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        label
            .frame(width: size, height: size)
            .contentShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .background(
                !isEnabled ? Color.clear
                    : isPressed ? MacDesignColor.pressed
                    : isHovered ? MacDesignColor.hover
                    : Color.clear,
                in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
            )
            .onHover { isHovered = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovered = false } }
            .opacity(isEnabled ? 1 : 0.45)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovered)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isPressed)
    }
}

private struct MacMenuHitExpandOverlay: NSViewRepresentable {
    func makeNSView(context: Context) -> HitExpandNSView {
        HitExpandNSView(frame: .zero)
    }

    func updateNSView(_ nsView: HitExpandNSView, context: Context) {}

    final class HitExpandNSView: NSView {
        override func isAccessibilityElement() -> Bool { false }

        override func hitTest(_ point: NSPoint) -> NSView? {
            guard !isHiddenOrHasHiddenAncestor else { return nil }
            let local = convert(point, from: superview)
            guard bounds.contains(local), let control = findWrappedControl() else { return nil }
            guard control.isEnabled else { return nil }
            let windowPoint = superview?.convert(point, to: nil) ?? .zero
            let controlWindowRect = control.convert(control.bounds, to: nil)
            if controlWindowRect.contains(windowPoint) {
                // Let clicks on the native NSPopUpButton itself pass through untouched.
                return nil
            }
            return self
        }

        override func mouseDown(with event: NSEvent) {
            guard let control = findWrappedControl(), control.isEnabled else {
                super.mouseDown(with: event)
                return
            }
            control.performClick(nil)
        }

        private func findWrappedControl() -> NSControl? {
            let targetRect = convert(bounds, to: nil)
            var container: NSView? = superview
            for _ in 0..<4 {
                guard let current = container else { break }
                if let match = findControl(in: current, withinWindowRect: targetRect) {
                    return match
                }
                container = current.superview
            }
            return nil
        }

        private func findControl(in view: NSView, withinWindowRect targetRect: NSRect) -> NSControl? {
            for subview in view.subviews {
                if subview === self || subview.isHidden { continue }
                if let control = subview as? NSControl, control is NSPopUpButton || control is NSButton {
                    let controlRect = control.convert(control.bounds, to: nil)
                    let center = NSPoint(x: controlRect.midX, y: controlRect.midY)
                    if targetRect.insetBy(dx: -3, dy: -3).contains(center),
                       targetRect.intersects(controlRect) {
                        return control
                    }
                }
                if let nested = findControl(in: subview, withinWindowRect: targetRect) {
                    return nested
                }
            }
            return nil
        }
    }
}

private struct MacComposerPillModifier: ViewModifier {
    var isActive = false
    var tint: Color = MacDesignColor.brand
    @State private var isHovered = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorSchemeContrast) private var contrast

    private var fillColor: Color {
        if isActive {
            return colorScheme == .dark
                ? Color.white.opacity(isEnabled && isHovered ? 0.14 : 0.10)
                : tint.opacity(isEnabled && isHovered ? 0.14 : 0.10)
        }
        if isEnabled && isHovered {
            return colorScheme == .dark ? Color.white.opacity(0.08) : MacDesignColor.hover
        }
        return .clear
    }

    private var borderColor: Color {
        if contrast == .increased {
            return Color.primary.opacity(0.35)
        }
        if isActive {
            return colorScheme == .dark ? Color.white.opacity(0.16) : tint.opacity(0.28)
        }
        return .clear
    }

    func body(content: Content) -> some View {
        content
            .background(
                fillColor,
                in: RoundedRectangle(cornerRadius: 8, style: .continuous)
            )
            .overlay {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .strokeBorder(borderColor, lineWidth: contrast == .increased ? 1 : 0.75)
            }
            .contentShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            .onHover { isHovered = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovered = false } }
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovered)
    }
}

struct MacSectionLabel: View {
    let title: LocalizedStringKey

    init(_ title: LocalizedStringKey) {
        self.title = title
    }

    var body: some View {
        Text(title)
            .font(.caption.weight(.semibold))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 2)
            .accessibilityAddTraits(.isHeader)
    }
}

struct MacPageMetric: Identifiable {
    let id: String
    let title: LocalizedStringKey
    let value: String
    let symbol: String
    let tint: Color

    init(
        id: String,
        _ title: LocalizedStringKey,
        value: String,
        symbol: String,
        tint: Color
    ) {
        self.id = id
        self.title = title
        self.value = value
        self.symbol = symbol
        self.tint = tint
    }
}

struct MacPageMetricsBar: View {
    let metrics: [MacPageMetric]
    var isWorking: Bool? = nil
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ViewThatFits(in: .horizontal) {
            wideMetrics
            compactMetrics
        }
        .padding(.trailing, isWorking == nil ? 0 : 24)
        .overlay(alignment: .trailing) {
            if isWorking == true {
                ProgressView()
                    .controlSize(.small)
                    .accessibilityLabel(String(localized: "Working"))
            }
        }
        .padding(.horizontal, MacDesignTokens.Spacing.md)
        .frame(minHeight: 46)
        .macPanel()
    }

    private var wideMetrics: some View {
        HStack(spacing: 0) {
            ForEach(Array(metrics.enumerated()), id: \.element.id) { index, metric in
                if index > 0 {
                    Divider()
                        .frame(height: 24)
                        .padding(.horizontal, MacDesignTokens.Spacing.md)
                }

                metricView(metric)
                    .frame(minWidth: 108, maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private var compactMetrics: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: MacDesignTokens.Spacing.xl) {
                ForEach(metrics) { metric in
                    metricView(metric)
                        .frame(minWidth: 112, alignment: .leading)
                }
            }
            .padding(.vertical, MacDesignTokens.Spacing.xs)
        }
    }

    private func metricView(_ metric: MacPageMetric) -> some View {
        HStack(spacing: MacDesignTokens.Spacing.sm) {
            Image(systemName: metric.symbol)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(metric.tint)
                .frame(width: 20, height: 20)
                .background(metric.tint.opacity(0.09), in: RoundedRectangle(cornerRadius: 5))
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 1) {
                Text(metric.value)
                    .font(.system(size: 13, weight: .semibold))
                    .monospacedDigit()
                    .contentTransition(.numericText())
                    .animation(
                        reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.selection),
                        value: metric.value
                    )
                Text(metric.title)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("page.metric." + metric.id)
    }
}

struct MacDesktopSplitView<Sidebar: View, Detail: View>: View {
    var sidebarMinWidth: CGFloat = 220
    var sidebarIdealWidth: CGFloat = MacDesignTokens.Layout.sidebarWidth
    var sidebarMaxWidth: CGFloat = 320
    @ViewBuilder let sidebar: () -> Sidebar
    @ViewBuilder let detail: () -> Detail

    var body: some View {
        GeometryReader { proxy in
            if proxy.size.width >= 760 {
                HSplitView {
                    sidebar()
                        .frame(
                            minWidth: sidebarMinWidth,
                            idealWidth: sidebarIdealWidth,
                            maxWidth: sidebarMaxWidth,
                            maxHeight: .infinity
                        )
                        .background(MacDesignColor.sidebar)

                    detail()
                        .frame(minWidth: 0, maxWidth: .infinity, maxHeight: .infinity)
                        .background(MacPageBackground())
                }
            } else {
                VStack(spacing: 0) {
                    sidebar()
                        .frame(maxWidth: .infinity, minHeight: 150, idealHeight: 190, maxHeight: 220)
                        .background(MacDesignColor.sidebar)
                    Divider()
                    detail()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .background(MacPageBackground())
                }
            }
        }
    }
}

struct MacIconTile: View {
    let symbol: String
    let tint: Color
    var size = MacDesignTokens.Control.featureIcon

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: 15, weight: .medium))
            .foregroundStyle(tint)
            .frame(width: size, height: size)
            .background(
                tint.opacity(0.085),
                in: RoundedRectangle(
                    cornerRadius: MacDesignTokens.Radius.field,
                    style: .continuous
                )
            )
            .accessibilityHidden(true)
    }
}

struct MacStatusBadge: View {
    let title: String
    let tint: Color
    var symbol: String? = nil

    var body: some View {
        HStack(spacing: MacDesignTokens.Spacing.xs) {
            if let symbol {
                Image(systemName: symbol)
                    .font(.system(size: 9.5, weight: .semibold))
                    .accessibilityHidden(true)
            } else {
                Circle()
                    .fill(tint)
                    .frame(width: 6, height: 6)
                    .accessibilityHidden(true)
            }
            Text(title)
                .lineLimit(1)
                .truncationMode(.middle)
        }
        .font(.caption.weight(.semibold))
        .foregroundStyle(tint)
        .padding(.horizontal, MacDesignTokens.Spacing.sm)
        .frame(height: 24)
        .background(
            tint.opacity(0.085),
            in: Capsule(style: .continuous)
        )
        .accessibilityElement(children: .combine)
        .accessibilityLabel(title)
    }
}

struct MacInlineNotice: View {
    enum Kind {
        case success
        case information
        case warning
        case failure

        var tint: Color {
            switch self {
            case .success: MacDesignColor.success
            case .information: MacDesignColor.brand
            case .warning: MacDesignColor.warning
            case .failure: MacDesignColor.danger
            }
        }

        var symbol: String {
            switch self {
            case .success: "checkmark.circle.fill"
            case .information: "info.circle.fill"
            case .warning: "exclamationmark.triangle.fill"
            case .failure: "xmark.octagon.fill"
            }
        }
    }

    var title: String? = nil
    let message: String
    let kind: Kind
    var actionTitle: String? = nil
    var action: (() -> Void)? = nil
    var dismiss: (() -> Void)? = nil
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(alignment: .center, spacing: MacDesignTokens.Spacing.sm) {
            Image(systemName: kind.symbol)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(kind.tint)
                .frame(width: 20, height: 20)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: title == nil ? 0 : 2) {
                if let title {
                    Text(title)
                        .font(.system(size: 11.5, weight: .semibold))
                }
                Text(message)
                    .font(.system(size: 11.5, weight: title == nil ? .medium : .regular))
                    .foregroundStyle(title == nil ? .primary : .secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Spacer(minLength: MacDesignTokens.Spacing.md)

            if let actionTitle, let action {
                Button(actionTitle, action: action)
                    .controlSize(.small)
            }

            if let dismiss {
                Button(action: dismiss) {
                    Image(systemName: "xmark")
                        .font(.system(size: 9, weight: .semibold))
                }
                .macToolbarIcon()
                .help(String(localized: "Dismiss"))
                .accessibilityLabel(String(localized: "Dismiss"))
            }
        }
        .padding(.leading, MacDesignTokens.Spacing.md)
        .padding(.trailing, dismiss == nil ? MacDesignTokens.Spacing.md : MacDesignTokens.Spacing.xs)
        .padding(.vertical, MacDesignTokens.Spacing.sm)
        .frame(maxWidth: .infinity, minHeight: 42, alignment: .leading)
        .background(
            kind.tint.opacity(0.075),
            in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.panel, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: MacDesignTokens.Radius.panel, style: .continuous)
                .stroke(kind.tint.opacity(0.16), lineWidth: 0.5)
        }
        .shadow(color: kind.tint.opacity(0.06), radius: 12, y: 4)
        .macDisclosureTransition(reduceMotion: reduceMotion)
        .accessibilityElement(children: .contain)
    }
}

struct MacEmptyState<Actions: View>: View {
    let title: LocalizedStringKey
    let description: LocalizedStringKey
    let symbol: String
    @ViewBuilder let actions: () -> Actions

    init(
        _ title: LocalizedStringKey,
        description: LocalizedStringKey,
        symbol: String,
        @ViewBuilder actions: @escaping () -> Actions
    ) {
        self.title = title
        self.description = description
        self.symbol = symbol
        self.actions = actions
    }

    var body: some View {
        VStack(spacing: MacDesignTokens.Spacing.lg) {
            Image(systemName: symbol)
                .font(.system(size: 21, weight: .medium))
                .foregroundStyle(MacDesignColor.brand.opacity(0.88))
                .frame(width: 48, height: 48)
                .background(
                    MacDesignColor.brand.opacity(0.085),
                    in: RoundedRectangle(
                        cornerRadius: MacDesignTokens.Radius.panel,
                        style: .continuous
                    )
                )
                .accessibilityHidden(true)

            VStack(spacing: MacDesignTokens.Spacing.xs) {
                Text(title)
                    .font(.title3.weight(.semibold))
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 420)
                Text(description)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: 420)
            }

            ViewThatFits(in: .horizontal) {
                HStack(spacing: MacDesignTokens.Spacing.sm) {
                    actions()
                }
                VStack(spacing: MacDesignTokens.Spacing.sm) {
                    actions()
                }
            }
            .buttonStyle(.bordered)
            .controlSize(.regular)
        }
        .padding(MacDesignTokens.Spacing.xxxl)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct MacLoadingState: View {
    let title: String

    init(_ title: String) {
        self.title = title
    }

    var body: some View {
        VStack(spacing: MacDesignTokens.Spacing.md) {
            ProgressView()
                .controlSize(.small)
            Text(title)
                .font(.system(size: 11.5, weight: .medium))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(MacDesignTokens.Spacing.xxl)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(title)
    }
}

/// Minimum interaction bounds live inside the button label, not outside its action.
struct MacActionButtonStyle: ButtonStyle {
    var isSelected = false
    var contentInsets = MacDesignTokens.Control.textInsets

    func makeBody(configuration: Configuration) -> some View {
        MacActionButtonBody(label: configuration.label, isPressed: configuration.isPressed, isSelected: isSelected, contentInsets: contentInsets)
    }
}

private struct MacActionButtonBody<Label: View>: View {
    let label: Label
    let isPressed: Bool
    let isSelected: Bool
    let contentInsets: EdgeInsets
    @State private var isHovered = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var fillColor: Color {
        if colorScheme == .dark {
            if isSelected { return Color.white.opacity(0.14) }
            if isEnabled && isPressed { return Color.white.opacity(0.15) }
            if isEnabled && isHovered { return Color.white.opacity(0.10) }
            return Color.white.opacity(0.055)
        }
        return isSelected ? MacDesignColor.selected : isEnabled && isPressed ? MacDesignColor.pressed : isEnabled && isHovered ? MacDesignColor.hover : MacDesignColor.panel
    }

    private var borderColor: Color {
        if colorScheme == .dark {
            if isSelected { return Color.white.opacity(0.22) }
            if isEnabled && isHovered { return Color.white.opacity(0.20) }
            return Color.white.opacity(0.10)
        }
        return isEnabled && isHovered ? MacDesignColor.controlOutline : MacDesignColor.borderSubtle
    }

    var body: some View {
        label
            .padding(contentInsets)
            .frame(minWidth: MacDesignTokens.Control.iconButton, minHeight: MacDesignTokens.Control.iconButton)
            .contentShape(Rectangle())
            .background(
                RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
                    .fill(fillColor)
                    .overlay(
                        RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous)
                            .strokeBorder(borderColor, lineWidth: colorScheme == .dark ? 0.75 : 1)
                    )
            )
            .foregroundStyle(isEnabled ? Color.primary : Color.secondary)
            .onHover { isHovered = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovered = false } }
            .opacity(isEnabled ? 1 : 0.45)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovered)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isPressed)
    }
}

/// Primary prominent CTA style that preserves WCAG AA contrast in both Light and Dark Mode.
struct NolaProminentButtonStyle: ButtonStyle {
    var isDestructive = false

    func makeBody(configuration: Configuration) -> some View {
        NolaProminentButtonBody(
            label: configuration.label,
            isPressed: configuration.isPressed,
            isDestructive: isDestructive || configuration.role == .destructive
        )
    }
}

extension ButtonStyle where Self == NolaProminentButtonStyle {
    static var nolaProminent: NolaProminentButtonStyle { NolaProminentButtonStyle() }
    static var nolaDestructiveProminent: NolaProminentButtonStyle { NolaProminentButtonStyle(isDestructive: true) }
}

private struct NolaProminentButtonBody<Label: View>: View {
    let label: Label
    let isPressed: Bool
    let isDestructive: Bool
    @State private var isHovered = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.controlSize) private var controlSize
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var insets: EdgeInsets {
        switch controlSize {
        case .mini: return EdgeInsets(top: 2, leading: 6, bottom: 2, trailing: 6)
        case .small: return EdgeInsets(top: 4, leading: 10, bottom: 4, trailing: 10)
        case .large: return EdgeInsets(top: 8, leading: 16, bottom: 8, trailing: 16)
        default: return EdgeInsets(top: 6, leading: 12, bottom: 6, trailing: 12)
        }
    }

    private var minHeight: CGFloat {
        switch controlSize {
        case .mini: return 18
        case .small: return 24
        case .large: return 34
        default: return 28
        }
    }

    private var cornerRadius: CGFloat {
        switch controlSize {
        case .mini: return 6
        case .small: return 7
        case .large: return MacDesignTokens.Radius.control
        default: return 8
        }
    }

    private var fillColor: Color {
        if colorScheme == .dark {
            if isDestructive {
                let base = Color(red: 0.84, green: 0.27, blue: 0.29)
                if !isEnabled { return base }
                if isPressed { return Color(red: 0.72, green: 0.22, blue: 0.24) }
                if isHovered { return Color(red: 0.90, green: 0.32, blue: 0.34) }
                return base
            }
            if !isEnabled { return Color.white.opacity(0.18) }
            if isPressed { return Color(red: 0.86, green: 0.86, blue: 0.88) }
            if isHovered { return Color.white }
            return Color(red: 0.96, green: 0.96, blue: 0.97)
        }
        let base = isDestructive ? MacDesignColor.danger : MacDesignColor.primaryAction
        if !isEnabled { return base }
        if isPressed { return base.opacity(0.80) }
        if isHovered { return base.opacity(0.90) }
        return base
    }

    private var foregroundColor: Color {
        if colorScheme == .dark {
            return isDestructive ? Color.white : Color(red: 0.04, green: 0.04, blue: 0.05)
        }
        return MacDesignColor.onPrimaryAction
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        label
            .fontWeight(.medium)
            .foregroundStyle(foregroundColor)
            .padding(insets)
            .frame(minHeight: minHeight)
            .background(
                shape
                    .fill(fillColor)
                    .overlay(
                        shape.strokeBorder(
                            LinearGradient(
                                colors: [Color.white.opacity(colorScheme == .dark ? 0.24 : 0.28), Color.white.opacity(0.02)],
                                startPoint: .top,
                                endPoint: .bottom
                            ),
                            lineWidth: 0.75
                        )
                    )
            )
            .contentShape(shape)
            .onHover { isHovered = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovered = false } }
            .opacity(isEnabled ? 1 : 0.45)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovered)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isPressed)
    }
}

/// Translucent obsidian-glass segmented tab bar that avoids neon accent-colored native picker blocks.
struct MacSegmentedTabBar<Option: Hashable>: View {
    let options: [Option]
    @Binding var selection: Option
    let title: (Option) -> String
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var pillFill: Color {
        colorScheme == .dark ? Color.white.opacity(0.13) : Color.white
    }

    private var pillBorder: Color {
        colorScheme == .dark ? Color.white.opacity(0.16) : MacDesignColor.borderSubtle
    }

    private var trackFill: Color {
        colorScheme == .dark ? Color.white.opacity(0.045) : MacDesignColor.panel
    }

    private var trackBorder: Color {
        colorScheme == .dark ? Color.white.opacity(0.08) : MacDesignColor.borderSubtle
    }

    private var trackBackground: some View {
        let shape = RoundedRectangle(cornerRadius: 10, style: .continuous)
        return shape
            .fill(trackFill)
            .overlay(shape.strokeBorder(trackBorder, lineWidth: 0.75))
    }

    @ViewBuilder
    private func segmentButton(for option: Option) -> some View {
        let isSelected = selection == option
        let pillShape = RoundedRectangle(cornerRadius: 7, style: .continuous)
        Button {
            withAnimation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.selection)) {
                selection = option
            }
        } label: {
            Text(title(option))
                .font(.system(size: 13, weight: isSelected ? .semibold : .medium))
                .foregroundStyle(isSelected ? Color.primary : Color.secondary)
                .padding(.horizontal, 14)
                .padding(.vertical, 6)
                .background {
                    if isSelected {
                        pillShape
                            .fill(pillFill)
                            .overlay(pillShape.strokeBorder(pillBorder, lineWidth: 0.75))
                    }
                }
                .contentShape(pillShape)
        }
        .buttonStyle(.plain)
    }

    var body: some View {
        HStack(spacing: 2) {
            ForEach(options, id: \.self) { option in
                segmentButton(for: option)
            }
        }
        .padding(3)
        .background(trackBackground)
    }
}

/// Unified full-width horizontal bar affordance for disclosures and expandable rows.
struct MacHorizontalBarButtonStyle: ButtonStyle {
    var isSubtle = true
    var isExpanded = false

    func makeBody(configuration: Configuration) -> some View {
        MacHorizontalBarButtonBody(
            label: configuration.label,
            isPressed: configuration.isPressed,
            isSubtle: isSubtle,
            isExpanded: isExpanded
        )
    }
}

private struct MacHorizontalBarButtonBody<Label: View>: View {
    let label: Label
    let isPressed: Bool
    let isSubtle: Bool
    let isExpanded: Bool
    @State private var isHovered = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorSchemeContrast) private var contrast

    private var fillColor: Color {
        if !isEnabled {
            return isSubtle ? MacDesignColor.subtleFill.opacity(0.45) : MacDesignColor.panel
        }
        if isPressed {
            return MacDesignColor.pressed
        }
        if isHovered {
            return MacDesignColor.hover
        }
        if isExpanded {
            return isSubtle ? MacDesignColor.subtleFill.opacity(0.95) : MacDesignColor.panel
        }
        return isSubtle ? MacDesignColor.subtleFill.opacity(0.72) : MacDesignColor.panel
    }

    private var borderColor: Color {
        if contrast == .increased {
            return Color.primary.opacity(0.45)
        }
        return (isEnabled && isHovered) ? MacDesignColor.controlOutline.opacity(0.65) : MacDesignColor.borderSubtle
    }

    var body: some View {
        label
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
            .contentShape(Rectangle())
            .background(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(fillColor)
                    .overlay {
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .strokeBorder(borderColor, lineWidth: contrast == .increased ? 1.2 : 0.75)
                    }
            )
            .foregroundStyle(isEnabled ? Color.primary : Color.secondary)
            .onHover { isHovered = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovered = false } }
            .opacity(isEnabled ? 1 : 0.45)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovered)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isPressed)
    }
}

private struct MacPlainDisclosureButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        MacPlainDisclosureButtonBody(label: configuration.label, isPressed: configuration.isPressed)
    }
}

private struct MacPlainDisclosureButtonBody<Label: View>: View {
    let label: Label
    let isPressed: Bool
    @State private var isHovered = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        label
            .padding(.horizontal, 6)
            .padding(.vertical, 4)
            .frame(maxWidth: .infinity, minHeight: 28, alignment: .leading)
            .contentShape(Rectangle())
            .background(
                !isEnabled ? Color.clear
                    : isPressed ? MacDesignColor.pressed
                    : isHovered ? MacDesignColor.hover
                    : Color.clear,
                in: RoundedRectangle(cornerRadius: 6, style: .continuous)
            )
            .foregroundStyle(isEnabled ? Color.primary : Color.secondary)
            .onHover { isHovered = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovered = false } }
            .opacity(isEnabled ? 1 : 0.45)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovered)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isPressed)
    }
}

/// The title, chevron and trailing whitespace are one accessible horizontal-bar disclosure action.
struct MacRowDisclosureStyle: DisclosureGroupStyle {
    var toggleIdentifier = ""
    var isPlain = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            disclosureButton(configuration: configuration)
                .accessibilityIdentifier(toggleIdentifier)
                .accessibilityValue(configuration.isExpanded ? Text("Expanded", tableName: "Chat") : Text("Collapsed", tableName: "Chat"))

            if configuration.isExpanded {
                configuration.content
                    .macDisclosureTransition(reduceMotion: reduceMotion)
            }
        }
    }

    @ViewBuilder
    private func disclosureButton(configuration: Configuration) -> some View {
        let button = Button {
            withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) {
                configuration.isExpanded.toggle()
            }
        } label: {
            HStack(spacing: 8) {
                configuration.label
                Spacer(minLength: 8)
                Image(systemName: "chevron.down")
                    .font(.system(size: 10.5, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(configuration.isExpanded ? 180 : 0))
                    .accessibilityHidden(true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }

        if isPlain {
            button.buttonStyle(MacPlainDisclosureButtonStyle())
        } else {
            button.buttonStyle(MacHorizontalBarButtonStyle(isSubtle: true, isExpanded: configuration.isExpanded))
        }
    }
}

struct MacDropdownOption: Identifiable {
    let id: String
    let title: String
    let isSelected: Bool
    var isAvailable: Bool = true
    let action: @MainActor () -> Void
}

/// Convenience wrapper binding a strongly typed `Selection` to `MacHorizontalDropdownBar`.
struct MacSelectionDropdownBar<Selection: Hashable>: View {
    var symbol: String? = nil
    let title: String
    var identifier = ""
    @Binding var selection: Selection
    let options: [(value: Selection, title: String, isEnabled: Bool)]

    init(
        symbol: String? = nil,
        title: String,
        identifier: String = "",
        selection: Binding<Selection>,
        options: [(value: Selection, title: String)]
    ) {
        self.symbol = symbol
        self.title = title
        self.identifier = identifier
        self._selection = selection
        self.options = options.map { ($0.value, $0.title, true) }
    }

    init(
        symbol: String? = nil,
        title: String,
        identifier: String = "",
        selection: Binding<Selection>,
        optionsWithAvailability: [(value: Selection, title: String, isEnabled: Bool)]
    ) {
        self.symbol = symbol
        self.title = title
        self.identifier = identifier
        self._selection = selection
        self.options = optionsWithAvailability
    }

    private var selectionTitle: String {
        options.first(where: { $0.value == selection })?.title ?? ""
    }

    var body: some View {
        MacHorizontalDropdownBar(
            symbol: symbol,
            title: title,
            selectionTitle: selectionTitle,
            identifier: identifier,
            options: options.enumerated().map { index, item in
                MacDropdownOption(
                    id: "\(index)-\(item.title)",
                    title: item.title,
                    isSelected: item.value == selection,
                    isAvailable: item.isEnabled
                ) {
                    guard item.isEnabled else { return }
                    selection = item.value
                }
            }
        )
    }
}

/// Unified full-width horizontal dropdown bar backed by a silent `NSPopUpButton` overlay.
struct MacHorizontalDropdownBar: View {
    var symbol: String? = nil
    let title: String
    let selectionTitle: String
    var identifier = ""
    let options: [MacDropdownOption]
    @State private var isHovered = false
    @State private var isPressed = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorSchemeContrast) private var contrast

    private var fillColor: Color {
        if !isEnabled {
            return MacDesignColor.subtleFill.opacity(0.45)
        }
        if isPressed {
            return MacDesignColor.pressed
        }
        if isHovered {
            return MacDesignColor.hover
        }
        return MacDesignColor.subtleFill.opacity(0.72)
    }

    private var borderColor: Color {
        if contrast == .increased {
            return Color.primary.opacity(0.45)
        }
        return (isEnabled && isHovered) ? MacDesignColor.controlOutline.opacity(0.65) : MacDesignColor.borderSubtle
    }

    var body: some View {
        ZStack {
            HStack(spacing: 9) {
                if let symbol {
                    Image(systemName: symbol)
                        .font(.system(size: MacDesignTokens.Typography.inspectorDetail, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 24, height: 24)
                        .background(
                            Color.primary.opacity(0.05),
                            in: RoundedRectangle(cornerRadius: 6, style: .continuous)
                        )
                }

                Text(title)
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(isEnabled ? Color.primary : Color.secondary)
                    .lineLimit(1)

                Spacer(minLength: 8)

                Text(selectionTitle)
                    .font(.system(size: 12.5))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)

                Image(systemName: "chevron.down")
                    .font(.system(size: 10.5, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(fillColor)
                    .overlay {
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .strokeBorder(borderColor, lineWidth: contrast == .increased ? 1.2 : 0.75)
                    }
            )
            .opacity(isEnabled ? 1 : 0.45)
            .accessibilityHidden(true)

            MacPopUpButtonOverlay(
                title: title,
                selectionTitle: selectionTitle,
                identifier: identifier,
                options: options,
                isEnabled: isEnabled,
                colorScheme: colorScheme,
                onHoverChange: { hovered in
                    isHovered = isEnabled && hovered
                },
                onMenuVisibilityChange: { open in
                    isPressed = isEnabled && open
                }
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
        .onChange(of: isEnabled) { _, enabled in
            if !enabled {
                isHovered = false
                isPressed = false
            }
        }
        .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovered)
        .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isPressed)
    }
}

private final class SilentPopUpButtonCell: NSPopUpButtonCell {
    override func draw(withFrame cellFrame: NSRect, in controlView: NSView) {}
    override func drawInterior(withFrame cellFrame: NSRect, in controlView: NSView) {}
    override func drawBezel(withFrame frame: NSRect, in controlView: NSView) {}
    override func drawFocusRingMask(withFrame cellFrame: NSRect, in controlView: NSView) {}
}

private final class SilentPopUpButton: NSPopUpButton {
    var onHoverChange: ((Bool) -> Void)?
    var onMenuVisibilityChange: ((Bool) -> Void)?
    private var trackingAreaRef: NSTrackingArea?

    override class var cellClass: AnyClass? {
        get { SilentPopUpButtonCell.self }
        set {}
    }

    override var intrinsicContentSize: NSSize {
        NSSize(width: NSView.noIntrinsicMetric, height: 36)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let trackingAreaRef {
            removeTrackingArea(trackingAreaRef)
        }
        let area = NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(area)
        trackingAreaRef = area
    }

    override func mouseEntered(with event: NSEvent) {
        onHoverChange?(true)
        super.mouseEntered(with: event)
    }

    override func mouseExited(with event: NSEvent) {
        onHoverChange?(false)
        super.mouseExited(with: event)
    }

    override func willOpenMenu(_ menu: NSMenu, with event: NSEvent) {
        menu.minimumWidth = max(160, bounds.width)
        onMenuVisibilityChange?(true)
        super.willOpenMenu(menu, with: event)
    }

    override func didCloseMenu(_ menu: NSMenu, with event: NSEvent?) {
        onMenuVisibilityChange?(false)
        super.didCloseMenu(menu, with: event)
    }
}

private struct MacPopUpButtonOverlay: NSViewRepresentable {
    let title: String
    let selectionTitle: String
    let identifier: String
    let options: [MacDropdownOption]
    let isEnabled: Bool
    let colorScheme: ColorScheme
    let onHoverChange: (Bool) -> Void
    let onMenuVisibilityChange: (Bool) -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> SilentPopUpButton {
        let cell = SilentPopUpButtonCell(textCell: "", pullsDown: false)
        let button = SilentPopUpButton(frame: .zero, pullsDown: false)
        button.cell = cell
        button.pullsDown = false
        button.isBordered = false
        button.bezelStyle = .regularSquare
        button.autoenablesItems = false
        button.preferredEdge = .maxY
        return button
    }

    func updateNSView(_ button: SilentPopUpButton, context: Context) {
        context.coordinator.options = options
        button.onHoverChange = onHoverChange
        button.onMenuVisibilityChange = onMenuVisibilityChange
        button.isEnabled = isEnabled
        button.appearance = NSAppearance(named: colorScheme == .dark ? .darkAqua : .aqua)
        button.setAccessibilityLabel(title)
        button.setAccessibilityValue(selectionTitle)
        button.setAccessibilityIdentifier(identifier)

        guard let menu = button.menu else { return }
        menu.autoenablesItems = false
        menu.minimumWidth = max(160, button.bounds.width)
        menu.removeAllItems()
        var selectedIndex: Int?
        for (index, option) in options.enumerated() {
            let item = NSMenuItem(
                title: option.title,
                action: #selector(Coordinator.selectOption(_:)),
                keyEquivalent: ""
            )
            item.target = context.coordinator
            item.tag = index
            item.isEnabled = option.isAvailable
            item.state = option.isSelected ? .on : .off
            menu.addItem(item)
            if option.isSelected && selectedIndex == nil {
                selectedIndex = index
            }
        }
        if let selectedIndex {
            button.selectItem(at: selectedIndex)
        } else {
            button.select(nil)
        }
    }

    @MainActor
    final class Coordinator: NSObject {
        var options: [MacDropdownOption] = []

        @objc func selectOption(_ sender: NSMenuItem) {
            let index = sender.tag
            guard options.indices.contains(index), options[index].isAvailable else { return }
            options[index].action()
        }
    }
}

/// Unified, high-contrast macOS pill switch replacing inconsistent AppKit switches and checkboxes.
struct MacSwitchToggleStyle: ToggleStyle {
    var showsLabel = true

    func makeBody(configuration: Configuration) -> some View {
        MacSwitchToggleBody(configuration: configuration, showsLabel: showsLabel)
    }
}

extension ToggleStyle where Self == MacSwitchToggleStyle {
    static var macSwitch: MacSwitchToggleStyle { MacSwitchToggleStyle(showsLabel: true) }
    static var macSwitchControl: MacSwitchToggleStyle { MacSwitchToggleStyle(showsLabel: false) }
}

private struct MacSwitchPressButtonStyle: ButtonStyle {
    let showsLabel: Bool
    let isOn: Bool
    @Binding var isHovered: Bool
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorSchemeContrast) private var contrast

    func makeBody(configuration: Configuration) -> some View {
        Group {
            if showsLabel {
                HStack(alignment: .center, spacing: 12) {
                    configuration.label
                        .frame(maxWidth: .infinity, alignment: .leading)
                    switchTrack(isPressed: configuration.isPressed)
                }
            } else {
                switchTrack(isPressed: configuration.isPressed)
                    .fixedSize()
            }
        }
        .contentShape(Rectangle())
        .onHover { isHovered = isEnabled && $0 }
        .onChange(of: isEnabled) { _, enabled in if !enabled { isHovered = false } }
        .opacity(isEnabled ? 1 : 0.52)
        .animation(reduceMotion ? nil : MacDesignTokens.Motion.responsive, value: isOn)
        .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: configuration.isPressed)
        .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovered)
    }

    private func switchTrack(isPressed: Bool) -> some View {
        let trackFill: Color = if isOn {
            (isEnabled && isHovered) ? MacDesignColor.switchTrackOnHover : MacDesignColor.switchTrackOn
        } else {
            (isEnabled && isHovered) ? MacDesignColor.switchTrackOffHover : MacDesignColor.switchTrackOff
        }
        let offStrokeColor: Color = contrast == .increased
            ? Color.primary.opacity(0.65)
            : MacDesignColor.switchBorderOff
        let thumbWidth: CGFloat = (isEnabled && isPressed) ? 18 : 16
        let thumbFill: Color = (colorScheme == .dark && isOn) ? MacDesignColor.onAccent : Color.white

        return ZStack(alignment: isOn ? .trailing : .leading) {
            Capsule(style: .circular)
                .fill(trackFill)
            if !isOn || contrast == .increased {
                Capsule(style: .circular)
                    .strokeBorder(
                        isOn ? Color.primary.opacity(0.45) : offStrokeColor,
                        lineWidth: contrast == .increased ? 1.2 : 0.8
                    )
            }
            Capsule(style: .circular)
                .fill(thumbFill)
                .frame(width: thumbWidth, height: 16)
                .shadow(
                    color: .black.opacity(colorScheme == .dark ? 0.34 : 0.16),
                    radius: 1.5,
                    y: 0.6
                )
                .padding(2)
        }
        .frame(width: 36, height: 20)
    }
}

private struct MacSwitchToggleBody: View {
    let configuration: ToggleStyleConfiguration
    let showsLabel: Bool
    @State private var isHovered = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Button {
            withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.responsive) {
                configuration.isOn.toggle()
            }
        } label: {
            configuration.label
        }
        .buttonStyle(
            MacSwitchPressButtonStyle(
                showsLabel: showsLabel,
                isOn: configuration.isOn,
                isHovered: $isHovered
            )
        )
        .disabled(!isEnabled)
        .accessibilityValue(Text(configuration.isOn ? String(localized: "Enabled") : String(localized: "Disabled")))
    }
}
