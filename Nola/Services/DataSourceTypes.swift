// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements DataSourceTypes logic in the iOS service layer.
// - Interfaces:
//   - DataSourceType
//   - DataSourceKind
//   - DataSourceCapability
// - APIs:
//   - none
//
//
//  DataSourceTypes.swift
//  Nola
//
//  Shared types for data source management.
//  Keep this file free of SwiftUI dependencies.
//

import Foundation

public enum DataSourceType: String, CaseIterable, Codable, Hashable, Sendable {
    case contacts
    case calendars
    case reminders
    case health
    case url

    public var displayName: String {
        switch self {
        case .contacts: return NSLocalizedString("autogen.DataSourceTypes.9070cb0e", comment: "AUTOGEN")
        case .calendars: return NSLocalizedString("autogen.DataSourceTypes.2ecbc116", comment: "AUTOGEN")
        case .reminders: return NSLocalizedString("autogen.DataSourceTypes.35cffa73", comment: "AUTOGEN")
        case .health: return NSLocalizedString("autogen.DataSourceTypes.28ff5ffe", comment: "AUTOGEN")
        case .url: return NSLocalizedString("data_source.url.display_name", value: "可读取数据源", comment: "Readable data source display name")
        }
    }

    /// SF Symbol name (no SwiftUI dependency).
    public var iconSystemName: String {
        switch self {
        case .contacts: return "person.2.fill"
        case .calendars: return "calendar"
        case .reminders: return "checklist"
        case .health: return "heart.fill"
        case .url: return "tray.full.fill"
        }
    }

    public var supportedEvents: String {
        switch self {
        case .contacts: return NSLocalizedString("autogen.DataSourceTypes.15e018f4", comment: "AUTOGEN")
        case .calendars: return NSLocalizedString("autogen.DataSourceTypes.1eb3ff07", comment: "AUTOGEN")
        case .reminders: return NSLocalizedString("autogen.DataSourceTypes.082fbef9", comment: "AUTOGEN")
        case .health: return NSLocalizedString("autogen.DataSourceTypes.2426a028", comment: "AUTOGEN")
        case .url: return NSLocalizedString("data_source.url.supported_events", value: "网站、搜索结果、RSS、iCalendar、JSON、文件/文件夹", comment: "Supported readable data source inputs")
        }
    }

    public var usageDescription: String {
        switch self {
        case .contacts:
            return NSLocalizedString("autogen.DataSourceTypes.5f03bd33", comment: "AUTOGEN")
        case .calendars:
            return NSLocalizedString("autogen.DataSourceTypes.3684772e", comment: "AUTOGEN")
        case .reminders:
            return NSLocalizedString("autogen.DataSourceTypes.b186e113", comment: "AUTOGEN")
        case .health:
            return NSLocalizedString("autogen.DataSourceTypes.18c3106d", comment: "AUTOGEN")
        case .url:
            return NSLocalizedString("data_source.url.usage_description", value: "连接任何 Nola 可以读取的来源作为资料：网站、搜索结果、订阅源、API、文件或文件夹。它们可以进入对话上下文，也可以被定时操作处理并通知结果。", comment: "Readable data source usage description")
        }
    }
}

public struct SystemDataSourceAccessPolicyStore {
    public static let shared = SystemDataSourceAccessPolicyStore(
        applicationRuntime: IOSApplicationRuntime.shared
    )

    private let applicationRuntime: IOSApplicationRuntime?

    private struct StoredPolicy: Codable {
        let schemaVersion: Int
        let policy: String
    }

    init(
        applicationRuntime: IOSApplicationRuntime?
    ) {
        self.applicationRuntime = applicationRuntime
    }

    public var configurableTypes: [DataSourceType] {
        [.contacts, .calendars, .reminders, .health]
    }

    public func policy(for type: DataSourceType) -> URLDataSourceAccessPolicy {
        guard type != .url else { return .conversationOptIn }
        if let setting = try? applicationRuntime?.productSetting(key: canonicalKey(for: type)),
           let data = setting.valueJSON.data(using: .utf8),
           let stored = try? JSONDecoder().decode(StoredPolicy.self, from: data),
           let policy = URLDataSourceAccessPolicy(rawValue: stored.policy) {
            return policy
        }
        return .conversationOptIn
    }

    public func setPolicy(_ policy: URLDataSourceAccessPolicy, for type: DataSourceType) {
        guard type != .url else { return }
        persistCanonical(policy, for: type)
    }

    public func conversationOverrideKey(for type: DataSourceType) -> String {
        "system:\(type.rawValue)"
    }

    private func canonicalKey(for type: DataSourceType) -> String {
        "data-source.system-policy.ios.\(type.rawValue)"
    }

    private func persistCanonical(_ policy: URLDataSourceAccessPolicy, for type: DataSourceType) {
        guard let applicationRuntime,
              let data = try? JSONEncoder().encode(StoredPolicy(schemaVersion: 1, policy: policy.rawValue)),
              let valueJSON = String(data: data, encoding: .utf8) else { return }
        _ = try? applicationRuntime.putProductSetting(
            key: canonicalKey(for: type),
            valueJSON: valueJSON,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
        )
    }

    public func isEnabledForConversation(type: DataSourceType, explicitOverride: Bool?) -> Bool {
        AgentCoreDataSourceResolver.resolve(
            AgentCoreDataSourceResolutionInput(
                sourceID: conversationOverrideKey(for: type),
                policy: policy(for: type).rawValue,
                conversationOverride: explicitOverride,
                contract: nil
            )
        )?.enabled == true
    }
}

extension URLDataSourceAccessPolicy {
    var systemImageName: String {
        switch self {
        case .defaultEnabled: return "checkmark.circle.fill"
        case .conversationOptIn: return "bubble.left.and.bubble.right.fill"
        case .askWhenRelevant: return "questionmark.bubble.fill"
        case .blocked: return "nosign"
        }
    }

    var summaryText: String {
        switch self {
        case .defaultEnabled:
            return NSLocalizedString(
                "data_source.policy.summary.default_enabled",
                value: "Added automatically to every conversation unless you turn it off for that conversation.",
                comment: "Default enabled data source policy summary"
            )
        case .conversationOptIn:
            return NSLocalizedString(
                "data_source.policy.summary.conversation_opt_in",
                value: "Available in conversation settings and only used when you select it.",
                comment: "Conversation opt-in data source policy summary"
            )
        case .askWhenRelevant:
            return NSLocalizedString(
                "data_source.policy.summary.ask_when_relevant",
                value: "Nola asks before using this source when it becomes relevant.",
                comment: "Ask when relevant data source policy summary"
            )
        case .blocked:
            return NSLocalizedString(
                "data_source.policy.summary.blocked",
                value: "Never exposed to conversations or automations.",
                comment: "Blocked data source policy summary"
            )
        }
    }
}

public enum DataSourceKind: String, CaseIterable, Codable, Hashable, Sendable {
    case website
    case webSearch
    case api
    case feed
    case calendarFeed
    case file
    case folder
    case localReminder
    case system
    case custom

    public var displayName: String {
        switch self {
        case .website:
            return NSLocalizedString("data_source.kind.website", value: "网站 / 网页", comment: "Website data source kind")
        case .webSearch:
            return NSLocalizedString("data_source.kind.web_search", value: "搜索结果", comment: "Web search data source kind")
        case .api:
            return NSLocalizedString("data_source.kind.api", value: "API / JSON", comment: "API data source kind")
        case .feed:
            return NSLocalizedString("data_source.kind.feed", value: "RSS / Atom", comment: "Feed data source kind")
        case .calendarFeed:
            return NSLocalizedString("data_source.kind.calendar_feed", value: "日历订阅", comment: "Calendar feed data source kind")
        case .file:
            return NSLocalizedString("data_source.kind.file", value: "文件", comment: "File data source kind")
        case .folder:
            return NSLocalizedString("data_source.kind.folder", value: "文件夹", comment: "Folder data source kind")
        case .localReminder:
            return NSLocalizedString("data_source.kind.local_reminder", value: "本地提醒", comment: "Local reminder source kind")
        case .system:
            return NSLocalizedString("data_source.kind.system", value: "系统数据", comment: "System data source kind")
        case .custom:
            return NSLocalizedString("data_source.kind.custom", value: "自定义来源", comment: "Custom readable data source kind")
        }
    }

    public var iconSystemName: String {
        switch self {
        case .website: return "globe"
        case .webSearch: return "magnifyingglass"
        case .api: return "curlybraces"
        case .feed: return "dot.radiowaves.left.and.right"
        case .calendarFeed: return "calendar.badge.clock"
        case .file: return "doc.text"
        case .folder: return "folder"
        case .localReminder: return "bell"
        case .system: return "iphone"
        case .custom: return "tray.full"
        }
    }

    public var exampleText: String {
        switch self {
        case .website:
            return NSLocalizedString("data_source.kind.website.example", value: "网页、文档站、公开页面", comment: "Website source examples")
        case .webSearch:
            return NSLocalizedString("data_source.kind.search.example", value: "Google/Bing/自定义搜索结果 URL 或 API", comment: "Search source examples")
        case .api:
            return NSLocalizedString("data_source.kind.api.example", value: "JSON API、Webhook 输出、结构化接口", comment: "API source examples")
        case .feed:
            return NSLocalizedString("data_source.kind.feed.example", value: "RSS、Atom、博客/新闻订阅", comment: "Feed source examples")
        case .calendarFeed:
            return NSLocalizedString("data_source.kind.calendar_feed.example", value: "iCalendar、ICS 订阅", comment: "Calendar feed examples")
        case .file:
            return NSLocalizedString("data_source.kind.file.example", value: "PDF、Markdown、文本、CSV 等本地文件", comment: "File source examples")
        case .folder:
            return NSLocalizedString("data_source.kind.folder.example", value: "可读取的本地文件夹或项目目录", comment: "Folder source examples")
        case .localReminder:
            return NSLocalizedString("data_source.kind.local_reminder.example", value: "只在本机触发的提醒/通知", comment: "Local reminder examples")
        case .system:
            return NSLocalizedString("data_source.kind.system.example", value: "通讯录、日历、提醒事项、健康", comment: "System source examples")
        case .custom:
            return NSLocalizedString("data_source.kind.custom.example", value: "任何 Nola 可以读取并转成上下文的来源", comment: "Custom source examples")
        }
    }
}

public enum DataSourceCapability: String, CaseIterable, Codable, Hashable, Sendable {
    case chatContext
    case scheduledOperation
    case notification
    case localOnly

    public var displayName: String {
        switch self {
        case .chatContext:
            return NSLocalizedString("data_source.capability.chat_context", value: "可加入对话上下文", comment: "Can be attached to chat context")
        case .scheduledOperation:
            return NSLocalizedString("data_source.capability.scheduled_operation", value: "可设置定时操作", comment: "Can be used by scheduled operations")
        case .notification:
            return NSLocalizedString("data_source.capability.notification", value: "可通知结果", comment: "Can notify results")
        case .localOnly:
            return NSLocalizedString("data_source.capability.local_only", value: "本地优先", comment: "Local-first capability")
        }
    }
}
