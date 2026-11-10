// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides shared, restrained chrome for top-level macOS feature pages.

import SwiftUI

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
        static let message: CGFloat = 16
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
        static let chatContentWidth: CGFloat = 880
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
    static let primaryAction = Color("NolaTextPrimary")
    static let onPrimaryAction = Color("NolaCanvas")
    static let surfaceRaised = Color("NolaSurfaceRaised")
    static let controlOutline = Color("NolaControlOutline")
    static let brand = Color("AccentColor")
    static let canvas = Color("NolaCanvas")
    static let chrome = Color("NolaCanvas")
    static let sidebar = Color("NolaCanvas")
    static let inspector = Color("NolaCanvas")
    static let sidebarGlass = Color("NolaCanvas").opacity(0.62)
    static let inspectorGlass = Color("NolaCanvas").opacity(0.68)
    static let panel = Color("NolaSurface")
    static let hover = Color("NolaControlHover")
    static let pressed = Color("NolaControlPressed")
    static let selected = Color("NolaAccentSubtle")
    static let borderSubtle = Color("NolaDivider")
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

struct MacPageHeader<Subtitle: View, Actions: View>: View {
    let title: LocalizedStringKey
    let symbol: String
    let navigationAction: (() -> Void)?
    let navigationSymbol: String
    let navigationLabel: LocalizedStringKey
    @Environment(\.macUsesGlobalWorkspaceChrome) private var usesGlobalWorkspaceChrome
    @ViewBuilder let subtitle: () -> Subtitle
    @ViewBuilder let actions: () -> Actions

    init(
        _ title: LocalizedStringKey,
        symbol: String,
        navigationAction: (() -> Void)? = nil,
        navigationSymbol: String = "sidebar.left",
        navigationLabel: LocalizedStringKey = "Toggle Sidebar",
        @ViewBuilder subtitle: @escaping () -> Subtitle,
        @ViewBuilder actions: @escaping () -> Actions
    ) {
        self.title = title
        self.symbol = symbol
        self.navigationAction = navigationAction
        self.navigationSymbol = navigationSymbol
        self.navigationLabel = navigationLabel
        self.subtitle = subtitle
        self.actions = actions
    }

    var body: some View {
        Group {
            if usesGlobalWorkspaceChrome {
                HStack(alignment: .top, spacing: 16) {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(title).font(.system(size: 20, weight: .semibold))
                        subtitle().font(.callout).foregroundStyle(.secondary)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                    actions()
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
                        .frame(height: 0.5)
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
                Image(systemName: navigationSymbol)
                    .font(.system(size: 12, weight: .semibold))
                    .frame(
                        width: MacDesignTokens.Control.iconButton,
                        height: MacDesignTokens.Control.iconButton
                    )
            }
            .macToolbarIcon()
            .help(navigationLabel)
            .accessibilityLabel(navigationLabel)
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
    var isWorking = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ViewThatFits(in: .horizontal) {
            wideMetrics
            compactMetrics
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

            if isWorking {
                ProgressView()
                    .controlSize(.small)
                    .padding(.leading, MacDesignTokens.Spacing.md)
                    .accessibilityLabel(String(localized: "Working"))
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
                if isWorking {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityLabel(String(localized: "Working"))
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
            case .success: .green
            case .information: MacDesignColor.brand
            case .warning: .orange
            case .failure: .red
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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        label
            .padding(contentInsets)
            .frame(minWidth: MacDesignTokens.Control.iconButton, minHeight: MacDesignTokens.Control.iconButton)
            .contentShape(Rectangle())
            .background(NolaSurfaceBackground(
                fill: isSelected ? MacDesignColor.selected : isEnabled && isPressed ? MacDesignColor.pressed : isEnabled && isHovered ? MacDesignColor.hover : MacDesignColor.panel,
                border: isEnabled && isHovered ? MacDesignColor.controlOutline : MacDesignColor.borderSubtle,
                cornerRadius: MacDesignTokens.Radius.control, raised: false))
            .foregroundStyle(isEnabled ? Color.primary : Color.secondary)
            .onHover { isHovered = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovered = false } }
            .opacity(isEnabled ? 1 : 0.45)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovered)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isPressed)
    }
}

/// The title, chevron and trailing whitespace are one accessible disclosure action.
struct MacRowDisclosureStyle: DisclosureGroupStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Button {
                withAnimation(reduceMotion ? nil : .easeOut(duration: 0.18)) {
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
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(MacActionButtonStyle())
            .accessibilityValue(Text(configuration.isExpanded ? "Expanded" : "Collapsed", tableName: "Chat"))
            if configuration.isExpanded { configuration.content }
        }
    }
}
