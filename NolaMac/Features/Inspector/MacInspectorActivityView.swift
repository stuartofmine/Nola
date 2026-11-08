// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents conversation tool availability and routes capability-management actions.

import Observation
import SwiftUI

struct MacInspectorActivityView: View {
    @Bindable var model: MacWorkspaceModel
    let chromeSnapshot: MacChromeBridgeSnapshot
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        MacInspectorSection(title: String(localized: "Tools"), showsDivider: false) {
            toolStatus(
                String(localized: "Files", table: "Chat"),
                symbol: "folder",
                status: authorizationStatus
            )
            toolStatus(
                String(localized: "Shell and Git", table: "Chat"),
                symbol: "terminal",
                status: authorizationStatus
            )
            Button {
                openWindow(id: "extensions-center")
            } label: {
                toolStatus(
                    String(localized: "Browser", table: "Chat"),
                    symbol: "network",
                    status: browserStatusTitle
                )
            }
            .buttonStyle(.plain)
            .help(browserStatusDetail)
            toolStatus(
                String(localized: "Computer use", table: "Chat"),
                symbol: "cursorarrow.motionlines",
                status: authorizationStatus
            )
            Text(String(localized: "Tool availability and permission are checked for each operation. Execution location does not grant access.", table: "Chat"))
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Divider()
            Button {
                openWindow(id: "capability-center")
            } label: {
                HStack {
                    Label(String(localized: "System Access"), systemImage: "checkmark.shield")
                    Spacer()
                    Image(systemName: "arrow.up.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(String(localized: "System Access"))
            Button {
                openWindow(id: "extensions-center")
            } label: {
                HStack {
                    Label(String(localized: "Skills and MCP"), systemImage: "shippingbox")
                    Spacer()
                    Image(systemName: "arrow.up.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(String(localized: "Skills and MCP"))
            Button {
                openWindow(id: "diagnostics-center")
            } label: {
                HStack {
                    Label(String(localized: "Diagnostics"), systemImage: "stethoscope")
                    Spacer()
                    Image(systemName: "arrow.up.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(String(localized: "Diagnostics"))
        }
    }
    private var authorizationStatus: String {
        String(localized: "Checked on use", table: "Chat")
    }

    private var browserStatusTitle: String {
        if model.selectedConversationBrowserSnapshot?.isConnected == true {
            return String(localized: "Browser session connected", table: "Chat")
        }
        if model.executionLocation == .thisMac && chromeSnapshot.isExtensionConnected {
            return String(localized: "Chrome connected", table: "Chat")
        }
        return authorizationStatus
    }

    private var browserStatusDetail: String {
        String(localized: "A browser connection is not an authorization. Managed browser and Chrome access are checked separately when used.", table: "Chat")
    }

    private func toolStatus(
        _ title: String,
        symbol: String,
        status: String
    ) -> some View {
        HStack(spacing: 9) {
            Image(systemName: symbol)
                .font(.system(size: 11, weight: .semibold))
                .frame(width: 26, height: 26)
                .foregroundStyle(Color.secondary)
                .background(
                    Color.primary.opacity(0.028),
                    in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                )
            Text(title)
            Spacer()
            Text(status)
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(Color.secondary)
        }
        .font(.system(size: 11.5))
        .frame(minHeight: 34)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(title))
        .accessibilityValue(Text(status))
    }
}
