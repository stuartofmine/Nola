// SPDX-License-Identifier: Apache-2.0

import Foundation

public struct AgentExecutionPresentation: Sendable, Equatable {
    public let title: String
    public let detail: String
    public let systemImage: String
    public let keepsDeviceDataLocal: Bool
}

public extension AgentCoreExecutionMode {
    func presentation(hostName: String? = nil) -> AgentExecutionPresentation {
        switch self {
        case .deviceLocal:
            return .init(
                title: NSLocalizedString("chat.execution.device", value: "On Device", comment: "Device-local agent execution label"),
                detail: NSLocalizedString("chat.execution.device.detail", value: "Model and tools stay on this device", comment: "Device-local execution detail"),
                systemImage: "iphone",
                keepsDeviceDataLocal: true
            )
        case .hostRemote:
            return .init(
                title: hostName?.trimmingCharacters(in: .whitespacesAndNewlines).nonEmpty
                    ?? NSLocalizedString("chat.execution.host", value: "Host", comment: "Host agent execution label"),
                detail: NSLocalizedString("chat.execution.host.detail", value: "The connected host runs the agent and tools", comment: "Host execution detail"),
                systemImage: "desktopcomputer",
                keepsDeviceDataLocal: false
            )
        case .hybrid:
            return .init(
                title: NSLocalizedString("chat.execution.hybrid", value: "Host + Device", comment: "Hybrid execution label"),
                detail: NSLocalizedString("chat.execution.hybrid.detail", value: "Device access is shared only after approval", comment: "Hybrid execution detail"),
                systemImage: "arrow.triangle.2.circlepath",
                keepsDeviceDataLocal: false
            )
        }
    }
}

private extension String {
    var nonEmpty: String? { isEmpty ? nil : self }
}

