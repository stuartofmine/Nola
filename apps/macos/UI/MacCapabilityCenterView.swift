// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents Nola's native macOS capabilities and their system-permission state.

import AppKit
import SwiftUI

private func macCapabilityText(_ key: String, _ fallback: String) -> String {
    let keyed = Bundle.main.localizedString(forKey: key, value: nil, table: nil)
    guard keyed == key else { return keyed }
    return Bundle.main.localizedString(forKey: fallback, value: fallback, table: nil)
}

struct MacCapabilityCenterView: View {
    @State private var service = MacCapabilityService()
    let navigationAction: (() -> Void)?

    init(navigationAction: (() -> Void)? = nil) {
        self.navigationAction = navigationAction
    }

    var body: some View {
        VStack(spacing: 0) {
            MacPageHeader(
                LocalizedStringKey(macCapabilityText("mac.capabilities.title", "System Access")),
                symbol: "wrench.and.screwdriver",
                navigationAction: navigationAction
            ) {
                Text(macCapabilityText("mac.capabilities.subtitle", "System access and tools available to Nola on this Mac"))
            } actions: {
                Button {
                    service.refresh()
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .macToolbarIcon()
                .help(macCapabilityText("mac.capabilities.refresh", "Refresh"))
                .accessibilityLabel(macCapabilityText("mac.capabilities.refresh", "Refresh"))
            }

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 10) {
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .firstTextBaseline, spacing: 12) {
                            accessSectionTitle
                            Spacer()
                            accessSummaryLabel
                        }
                        VStack(alignment: .leading, spacing: 7) {
                            accessSectionTitle
                            accessSummaryLabel
                        }
                    }
                    ForEach(sortedCapabilities) { capability in
                        capabilityRow(capability)
                    }

                    HStack(alignment: .top, spacing: 11) {
                        Image(systemName: "info.circle.fill")
                            .foregroundStyle(MacDesignColor.brand)
                            .padding(.top, 1)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(macCapabilityText("mac.capabilities.permission_note.title", "Permission status uses runtime access"))
                                .font(.system(size: 13, weight: .semibold))
                            Text(macCapabilityText("mac.capabilities.permission_note.detail", "After changing macOS settings, return to Nola and refresh. The status reflects whether the current build can actually use the capability, not only whether a switch appears enabled."))
                                .font(.system(size: 12))
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .padding(14)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(MacDesignColor.brand.opacity(0.055), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                }
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.vertical, MacDesignTokens.Spacing.xl)
                .frame(maxWidth: MacDesignTokens.Layout.contentWidth)
                .frame(maxWidth: .infinity)
            }
        }
        .background(MacPageBackground())
        .task {
            service.refresh()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            service.refreshAfterAppActivation()
        }
    }

    private var unauthorizedCount: Int {
        service.capabilities.filter { $0.state == .permissionRequired }.count
    }

    private var accessSectionTitle: some View {
        Text(macCapabilityText("mac.capabilities.section", "Permissions & Access"))
            .font(.system(size: 12.5, weight: .semibold))
            .foregroundStyle(.secondary)
    }

    private var accessSummaryLabel: some View {
        Label(accessSummary, systemImage: unauthorizedCount == 0 ? "checkmark.circle" : "lock")
            .font(.system(size: 11.5, weight: .medium))
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
    }

    private var accessSummary: String {
        if unauthorizedCount == 0 {
            return macCapabilityText("mac.capabilities.access_available", "System access is available")
        }
        return String(
            format: macCapabilityText("mac.capabilities.available_on_request", "%d permissions can be requested when needed"),
            unauthorizedCount
        )
    }

    private var sortedCapabilities: [MacCapabilitySnapshot] {
        // Permission changes must not reorder a row while the user interacts with it.
        service.capabilities
    }

    private func capabilityRow(_ capability: MacCapabilitySnapshot) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 13) {
                capabilityIdentity(capability)
                Spacer(minLength: 16)
                capabilityControls(capability)
            }

            VStack(alignment: .leading, spacing: 12) {
                capabilityIdentity(capability)
                capabilityControls(capability)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(16)
        .macPanel()
    }

    private func capabilityIdentity(_ capability: MacCapabilitySnapshot) -> some View {
        HStack(spacing: 13) {
            MacIconTile(symbol: capability.symbol, tint: statusColor(capability.state))
            VStack(alignment: .leading, spacing: 4) {
                Text(capability.title)
                    .font(.system(size: 14, weight: .semibold))
                Text(capability.detail)
                    .font(.system(size: 12.5))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    @ViewBuilder
    private func capabilityControls(_ capability: MacCapabilitySnapshot) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 10) {
                capabilityStatus(capability)
                capabilityAction(capability)
            }

            VStack(alignment: .leading, spacing: 8) {
                capabilityStatus(capability)
                capabilityAction(capability)
            }
        }
    }

    private func capabilityStatus(_ capability: MacCapabilitySnapshot) -> some View {
        MacStatusBadge(
            title: capability.state == .permissionRequired
                ? macCapabilityText("mac.capabilities.not_authorized", "Not authorized")
                : capability.state.title,
            tint: statusColor(capability.state),
            symbol: statusSymbol(capability.state)
        )
    }

    @ViewBuilder
    private func capabilityAction(_ capability: MacCapabilitySnapshot) -> some View {
        if capability.hasUserAction {
            if capability.state == .permissionRequired {
                Button {
                    service.request(capability.kind)
                } label: {
                    Label(macCapabilityText("mac.capabilities.review_access", "Review Access"), systemImage: "lock.open")
                }
                .controlSize(.regular)
                .buttonStyle(.bordered)
            } else {
                Button {
                    service.request(capability.kind)
                } label: {
                    Label(macCapabilityText("mac.capabilities.open_settings", "Open Settings"), systemImage: "gear")
                }
                .controlSize(.regular)
                .buttonStyle(.bordered)
            }
        }
    }

    private func statusSymbol(_ state: MacCapabilityState) -> String {
        switch state {
        case .ready: return "checkmark.circle.fill"
        case .permissionRequired: return "lock"
        case .optional: return "circle.dashed"
        }
    }

    private func statusColor(_ state: MacCapabilityState) -> Color {
        switch state {
        case .ready: return .green
        case .permissionRequired: return .secondary
        case .optional: return .secondary
        }
    }
}
