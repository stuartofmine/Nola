// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides reusable in-progress status views for macOS chat presentation.

import SwiftUI

/// Reply actions remain unboxed; foreground and press feedback retain their
/// affordance without adding a panel beneath the answer.
struct MacMessageActionButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        MacMessageActionButtonBody(label: configuration.label, isPressed: configuration.isPressed)
    }
}

private struct MacMessageActionButtonBody<Label: View>: View {
    let label: Label
    let isPressed: Bool
    @State private var isHovering = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        label
            .frame(minWidth: 32, minHeight: 32)
            .contentShape(Rectangle())
            .foregroundStyle(isEnabled && (isHovering || isPressed) ? Color.primary : Color.secondary)
            .opacity(!isEnabled ? 0.45 : isPressed ? 0.65 : 1)
            .onHover { isHovering = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovering = false } }
            .animation(reduceMotion ? nil : .easeOut(duration: 0.10), value: isHovering)
            .animation(reduceMotion ? nil : .easeOut(duration: 0.10), value: isPressed)
    }
}

struct MacPressFeedbackButtonStyle: ButtonStyle {
    var contentInsets = MacDesignTokens.Control.textInsets

    func makeBody(configuration: Configuration) -> some View {
        MacPressFeedbackButtonBody(label: configuration.label, isPressed: configuration.isPressed, contentInsets: contentInsets)
    }
}

private struct MacPressFeedbackButtonBody<Label: View>: View {
    let label: Label
    let isPressed: Bool
    let contentInsets: EdgeInsets
    @State private var isHovering = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        label
            .padding(contentInsets)
            .frame(minWidth: MacDesignTokens.Control.iconButton, minHeight: MacDesignTokens.Control.iconButton)
            .contentShape(Rectangle())
            .background(isEnabled && isPressed ? MacDesignColor.pressed : isEnabled && isHovering ? MacDesignColor.hover : Color.clear,
                in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.control, style: .continuous))
            .opacity(isEnabled && isPressed ? 0.82 : 1)
            .onHover { isHovering = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovering = false } }
            .animation(
                reduceMotion ? nil : .timingCurve(0.2, 0.8, 0.2, 1, duration: 0.12),
                value: isPressed
            )
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovering)
            .contentShape(Rectangle())
    }
}

struct MacBreathingStatusLight: View {
    let diameter: CGFloat
    var dotScale: CGFloat = 1
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @Environment(\.scenePhase) private var scenePhase
    @State private var isVisible = false

    init(diameter: CGFloat = 28, dotScale: CGFloat = 1) {
        self.diameter = diameter
        self.dotScale = dotScale
    }

    var body: some View {
        // A status light must not become the most animated object in a reading
        // surface. Keep geometry fixed and invalidate only this small subtree.
        let animates = macContinuousStatusAnimationEnabled(
            isActive: scenePhase == .active,
            reduceMotion: reduceMotion,
            isVisible: isVisible
        )
        TimelineView(.animation(minimumInterval: 1.0 / 20, paused: !animates)) { context in
            let phase = !animates ? 1.0 : (sin(context.date.timeIntervalSinceReferenceDate * .pi / 1.2) + 1) / 2
            Circle()
                .fill(Color.accentColor)
                .opacity(0.65 + 0.35 * phase)
                .frame(width: diameter * 0.32 * dotScale, height: diameter * 0.32 * dotScale)
        }
        .frame(width: diameter, height: diameter)
        .onAppear { isVisible = true }
        .onDisappear { isVisible = false }
        .accessibilityHidden(true)
    }
}

struct MacAssistantPendingProcessView: View {
    let activities: [MacAgentActivity]
    let showsAnimatedStatusLight: Bool

    var body: some View {
        HStack(spacing: 9) {
            if showsAnimatedStatusLight {
                MacBreathingStatusLight(diameter: 16)
                    .frame(width: 16, height: 16)
            } else {
                ProgressView()
                    .controlSize(.small)
                    .frame(width: 16, height: 16)
            }

            if let activity = activities.last {
                VStack(alignment: .leading, spacing: 2) {
                    Text(activity.title)
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    if !activity.detail.isEmpty,
                       activity.detail != activity.title {
                        Text(activity.detail)
                            .font(.system(size: 13))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityLabel(String(localized: "Nola is processing"))
    }
}
