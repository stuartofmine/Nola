// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements Localizable logic in the iOS service layer.
// - Interfaces:
//   - L10n
// - APIs:
//   - none
//
//
//  Localizable.swift
//  Nola
//
//  Centralized localization system supporting multiple languages.
//

import Foundation
import SwiftUI

/// Localization manager for the app.
public final class L10n {
    /// Current language (follows system settings by default).
    nonisolated public static var currentLanguage: String {
        Locale.current.language.languageCode?.identifier ?? "en"
    }
    
    /// Localized string lookup.
    nonisolated public static func string(_ key: String, comment: String = "") -> String {
        NSLocalizedString(key, comment: comment)
    }

    /// Localized format string lookup.
    nonisolated public static func format(_ key: String, _ args: CVarArg..., comment: String = "") -> String {
        String(format: NSLocalizedString(key, comment: comment), arguments: args)
    }
}

// MARK: - Localization Keys

extension L10n {
    // MARK: - App Name & Common
    public static let appName = string("app.name", comment: "Application name")
    public static let ok = string("common.ok")
    public static let create = string("common.create")
    public static let cancel = string("common.cancel")
    public static let confirm = string("common.confirm")
    public static let done = string("common.done")
    public static let back = string("common.back")
    public static let yes = string("common.yes")
    public static let no = string("common.no")
    public static let skip = string("common.skip")
    public static let next = string("common.next")
    public static let previous = string("common.previous")
    public static let close = string("common.close")
    public static let commonActions = string("common.actions")
    public static let settings = string("common.settings")
    
    // MARK: - Onboarding
    public static let onboardingWelcomeTitle = string("onboarding.welcome.title")
    public static let onboardingWelcomeSubtitle = string("onboarding.welcome.subtitle")
    public static let onboardingStart = string("onboarding.start")
    public static let onboardingComplete = string("onboarding.complete")
    
    public static let onboardingFeature1 = string("onboarding.feature.1")
    public static let onboardingFeature2 = string("onboarding.feature.2")
    public static let onboardingFeature3 = string("onboarding.feature.3")
    public static let onboardingFeature4 = string("onboarding.feature.4")
    
    public static let onboardingPermissionPrompt = string("onboarding.permission.prompt")
    
    // MARK: - Permissions
    public static let permissionContactsTitle = string("permission.contacts.title")
    public static let permissionContactsDescription = string("permission.contacts.description")
    public static let permissionContactsBenefit1 = string("permission.contacts.benefit.1")
    public static let permissionContactsBenefit2 = string("permission.contacts.benefit.2")
    public static let permissionContactsBenefit3 = string("permission.contacts.benefit.3")
    
    public static let permissionCalendarsTitle = string("permission.calendars.title")
    public static let permissionCalendarsDescription = string("permission.calendars.description")
    public static let permissionCalendarsBenefit1 = string("permission.calendars.benefit.1")
    public static let permissionCalendarsBenefit2 = string("permission.calendars.benefit.2")
    public static let permissionCalendarsBenefit3 = string("permission.calendars.benefit.3")
    
    public static let permissionRemindersTitle = string("permission.reminders.title")
    public static let permissionRemindersDescription = string("permission.reminders.description")
    public static let permissionRemindersBenefit1 = string("permission.reminders.benefit.1")
    public static let permissionRemindersBenefit2 = string("permission.reminders.benefit.2")
    public static let permissionRemindersBenefit3 = string("permission.reminders.benefit.3")
    
    public static let permissionNotificationsTitle = string("permission.notifications.title")
    public static let permissionNotificationsDescription = string("permission.notifications.description")
    public static let permissionNotificationsBenefit1 = string("permission.notifications.benefit.1")
    public static let permissionNotificationsBenefit2 = string("permission.notifications.benefit.2")
    public static let permissionNotificationsBenefit3 = string("permission.notifications.benefit.3")
    
    public static let permissionGrant = string("permission.grant")
    public static let permissionGranted = string("permission.granted")
    public static let permissionDenied = string("permission.denied")
    public static let permissionRequesting = string("permission.requesting")
    public static let permissionGoToSettings = string("permission.go_to_settings")
    public static let permissionCanChangeAnytime = string("permission.can_change_anytime")

    // Additional permission-related strings
    public static let permissionFullAccessNote = string("permission.full_access_note")

    // Health specific
    public static let permissionHealthTitle = string("permission.health.title")
    public static let permissionHealthDescription = string("permission.health.description")
    public static let permissionHealthBenefit1 = string("permission.health.benefit.1")
    public static let permissionHealthBenefit2 = string("permission.health.benefit.2")
    
    // MARK: - Data Source Placeholders
    public static let dataSourceContactsBirthdayTitle = string("data_source.contacts.birthday_title")
    public static let dataSourceCalendarUntitledEvent = string("data_source.calendar.untitled_event")
    public static let dataSourceReminderUntitledReminder = string("data_source.reminder.untitled_reminder")

    // MARK: - URL Fetcher Descriptions
    public static let urlFetcherJsonDescription = string("url_fetcher.json.description")
    public static let urlFetcherIcalDescription = string("url_fetcher.ical.description")
    public static let urlFetcherRssDescription = string("url_fetcher.rss.description")
    public static let urlFetcherHtmlDescription = string("url_fetcher.html.description")
    public static let urlFetcherErrorParseFailed = string("url_fetcher.error.parse_failed")
    public static let urlFetcherErrorUnknownFormat = string("url_fetcher.error.unknown_format")
    public static let urlFetcherFormatJsonFound = string("url_fetcher.format.json_found")
    public static let urlFetcherFormatIcalFound = string("url_fetcher.format.ical_found")
    public static let urlFetcherFormatRssFound = string("url_fetcher.format.rss_found")
    public static let urlFetcherFormatHtmlFound = string("url_fetcher.format.html_found")
    public static let permissionHealthBenefit3 = string("permission.health.benefit.3")
    public static let permissionHealthCapabilityNote = string("permission.health.capability_note")
    public static let permissionHealthAuthorizationPopup = string("permission.health.authorization_popup_message")

    // Insufficient permission messages
    public static let permissionInsufficientTitle = string("permission.insufficient.title")
    public static let permissionInsufficientDescription = string("permission.insufficient.description")
    public static let permissionUpgradeToFullAccess = string("permission.upgrade_to_full_access")

    // Permissions setup view
    public static let permissionsSetupContactsDesc = string("permissions.setup.contacts.desc")
    public static let permissionsSetupCalendarsDesc = string("permissions.setup.calendars.desc")
    public static let permissionsSetupRemindersDesc = string("permissions.setup.reminders.desc")
    public static let permissionsSetupHealthDesc = string("permissions.setup.health.desc")
    public static let permissionsSetupFinishSetup = string("permissions.setup.finish_setup")
    public static let permissionsStartUsingNola = string("permissions.start_using_nola")
    public static let permissionsRequestFailed = string("permissions.request_failed")
    public static let permissionsUnknownError = string("permissions.unknown_error")
    public static let permissionsGoToSettings = permissionGoToSettings
    public static let permissionsSetupCompletedTitle = string("permissions.setup.completed.title")
    public static let permissionsSetupInfoAllGrantedTitle = string("permissions.setup.info.all_granted.title")
    public static let permissionsSetupInfoAllGrantedDesc = string("permissions.setup.info.all_granted.desc")
    public static let permissionsSetupInfoSomeGrantedTitle = string("permissions.setup.info.some_granted.title")
    public static let permissionsSetupInfoSomeGrantedDesc = string("permissions.setup.info.some_granted.desc")
    public static let permissionsSetupInfoNoneGrantedTitle = string("permissions.setup.info.none_granted.title")
    public static let permissionsSetupInfoNoneGrantedDesc = string("permissions.setup.info.none_granted.desc")
    public static let permissionsSetupFeaturePrivacyTitle = string("permissions.setup.feature.privacy.title")
    public static let permissionsSetupFeaturePrivacyDesc = string("permissions.setup.feature.privacy.desc")
    public static let permissionsSetupFeatureNoUploadTitle = string("permissions.setup.feature.no_upload.title")
    public static let permissionsSetupFeatureNoUploadDesc = string("permissions.setup.feature.no_upload.desc")
    public static let permissionsSetupFeatureControlTitle = string("permissions.setup.feature.control.title")
    public static let permissionsSetupFeatureControlDesc = string("permissions.setup.feature.control.desc")
    
    // MARK: - Home
    public static let homeTitle = string("home.title")
    public static let chatTitle = string("chat.title")
    public static let chatConversationDefaultTitle = string("chat.conversation.default_title")
    public static let chatConversationRecoveredTitle = string("chat.conversation.recovered_title")
    public static let chatTaskAcknowledgement = string("chat.task_acknowledgement")
    public static let chatErrorNoModelSelected = string("chat.error.no_model_selected")
    public static let chatErrorUnknown = string("chat.error.unknown")
    public static let homeGenerateSummary = string("home.generate_summary")
    public static let homeEmptyState = string("home.empty_state")
    public static let homeAnalyzing = string("home.analyzing")
    public static let homeSectionToday = string("home.section.today")
    public static let homeErrorNoEvents = string("home.error.no_events")
    public static let homeErrorNoImportantEvents = string("home.error.no_important_events")
    public static let homeErrorGenerationFailed = string("home.error.generation_failed")
    // Greeting strings
    public static let homeGreetingMorning = string("home.greeting.morning")
    public static let homeGreetingNoon = string("home.greeting.noon")
    public static let homeGreetingEvening = string("home.greeting.evening")
    public static let homeGreetingGeneric = string("home.greeting.generic")
    public static let homeGreetingAfternoon = string("home.greeting.afternoon")
    public static let homeGreetingNight = string("home.greeting.night")
    public static let homeErrorGenerationFailedDetail = string("home.error.generation_failed.detail")

    // Home additional (MainTabView)
    public static let homeSectionDataSources = string("home.section.data_sources")
    public static let homeSectionURLContent = string("home.section.url_content")
    public static let homeSectionCalendar = string("home.section.calendar")
    public static let homeSectionReminders = string("home.section.reminders")
    public static let homeSectionHealth = string("home.section.health")
    public static let homeSectionDataSourceDetails = string("home.section.data_source_details")

    public static let homeDataSourceLocalStorageLabel = string("home.data_source.local_storage")
    public static let homeDataSourcePrivacyDescription = string("home.data_source.privacy_description")

    public static let homeDataSourceStatusTitle = string("home.data_source.status.title")
    public static let homeDataSourceStatusSubtitle = string("home.data_source.status.subtitle")
    public static let homeOverviewEnabled = string("home.overview.enabled")
    public static let homeOverviewSyncing = string("home.overview.syncing")
    public static let homeOverviewNeedsAuth = string("home.overview.needs_auth")

    public static let homeStatusNotFetched = string("home.status.not_fetched")

    public static let homeDataSourceStatusStatSuccessful = string("home.data_source.stat.successful")
    public static let homeDataSourceStatusStatFailed = string("home.data_source.stat.failed")
    public static let homeDataSourceStatusStatEvents = string("home.data_source.stat.events")

    public static let homePriorityHigh = string("home.priority.high")
    public static let homePriorityMedium = string("home.priority.medium")
    public static let homePriorityLow = string("home.priority.low")

    public static let homeExplainNoModel = string("home.explain.no_model")
    
    public static let homePermissionAlertTitle = string("home.permission_alert.title")
    public static let homePermissionAlertMessage = string("home.permission_alert.message")
    public static let homePermissionAlertGoToSettings = string("home.permission_alert.go_to_settings")
    public static let homePermissionAlertCancel = string("home.permission_alert.cancel")
    public static let homeAlertOK = string("home.alert.ok")
    
    // MARK: - Settings
    public static let settingsSectionLLM = string("settings.section.llm")
    public static let settingsBaseURL = string("settings.base_url")
    public static let settingsAPIKey = string("settings.api_key")
    public static let settingsModel = string("settings.model")
    public static let settingsSaveAndTest = string("settings.save_and_test")
    public static let settingsTestConnection = string("settings.test_connection")
    public static let settingsReset = string("settings.reset")
    public static let settingsUseDevEnv = string("settings.use_dev_env")
    public static let settingsConfigSource = string("settings.config_source")
    public static let settingsEnvDebugTitle = string("settings.env_debug_title")
    public static let settingsDevEnvNoVars = string("settings.dev_env_no_vars")
    public static let settingsDevEnvApplied = string("settings.dev_env_applied")
    public static let settingsDevEnvPartial = string("settings.dev_env_partial")
    
    
    public static let settingsSectionLogs = string("settings.section.logs")
    public static let settingsLogLevel = string("settings.log_level")
    public static let settingsClearLogs = string("settings.clear_logs")
    public static let settingsNoLogs = string("settings.no_logs")
    
    public static let settingsSectionDataSources = string("settings.section.data_sources")
    public static let settingsDataSourceContacts = string("settings.data_source.contacts")
    public static let settingsDataSourceCalendars = string("settings.data_source.calendars")
    public static let settingsDataSourceReminders = string("settings.data_source.reminders")
    public static let dataSourceContactsSubtitle = string("datasource.contacts.subtitle")
    public static let dataSourceCalendarsSubtitle = string("datasource.calendars.subtitle")
    public static let dataSourceRemindersSubtitle = string("datasource.reminders.subtitle")

    // MARK: - Data Source Detail
    public static let dataSourceDetailSectionPermissionSettings = string("data_source_detail.section.permission_settings")
    public static let dataSourceDetailOpenSystemSettings = string("data_source_detail.open_system_settings")
    public static let dataSourceDetailSectionControl = string("data_source_detail.section.control")
    public static let dataSourceDetailEnableToggle = string("data_source_detail.enable_toggle")
    public static let dataSourceDetailRefreshing = string("data_source_detail.refreshing")
    public static let dataSourceDetailRefreshNow = string("data_source_detail.refresh_now")
    public static let dataSourceDetailSectionHealthOverview = string("data_source_detail.section.health_overview")
    public static let dataSourceDetailSectionInfo = string("data_source_detail.section.info")
    public static let dataSourceDetailInfoType = string("data_source_detail.info.type")
    public static let dataSourceDetailInfoSupportedEvents = string("data_source_detail.info.supported_events")
    public static let dataSourceDetailInfoEventCount = string("data_source_detail.info.event_count")
    public static let dataSourceDetailInfoLastUpdated = string("data_source_detail.info.last_updated")
    public static let dataSourceDetailSectionUsage = string("data_source_detail.section.usage")
    public static let dataSourceDetailSectionError = string("data_source_detail.section.error")
    public static let dataSourceDetailPermissionAlertTitle = string("data_source_detail.permission_alert.title")
    public static let dataSourceDetailErrorAlertTitle = string("data_source_detail.error_alert.title")
    public static let dataSourceDetailUnknownError = string("data_source_detail.error_unknown")
    public static let dataSourceDetailErrorHealthNotInitialized = string("data_source_detail.error.health_not_initialized")
    public static let dataSourceDetailStatusDisabled = string("data_source_detail.status.disabled")
    public static let dataSourceDetailStatusOK = string("data_source_detail.status.ok")
    public static let dataSourceDetailStatusInsufficient = string("data_source_detail.status.insufficient")
    public static let dataSourceDetailStatusDenied = string("data_source_detail.status.denied")
    public static let dataSourceDetailStatusNeedsAuth = string("data_source_detail.status.needs_auth")
    public static let dataSourceDetailStatPermissionStatus = string("data_source_detail.stat.permission_status")
    public static let dataSourceDetailStatSource = string("data_source_detail.stat.source")
    public static let dataSourceDetailStatEnabledValue = string("data_source_detail.stat.enabled")
    public static let dataSourceDetailStatDisabledValue = string("data_source_detail.stat.disabled")
    public static let dataSourceDetailStatEventCount = string("data_source_detail.stat.event_count")
    public static let dataSourceDetailPermissionAccessTitle = string("data_source_detail.permission.access_title")
    public static let dataSourceDetailPermissionRequestButton = string("data_source_detail.permission.request_button")
    public static let dataSourceDetailHealthMetricSteps = string("data_source_detail.health_metric.steps")
    public static let dataSourceDetailHealthMetricHeartRate = string("data_source_detail.health_metric.heart_rate")
    public static let dataSourceDetailHealthMetricSleep = string("data_source_detail.health_metric.sleep")
    public static let dataSourceDetailHealthMetricWorkout = string("data_source_detail.health_metric.workout")
    public static let homeHealthAverageSevenDays = string("home.health.average_7d")
    public static let homeHealthUnitBPM = string("home.health.unit.bpm")
    public static let homeHealthUnitHours = string("home.health.unit.hours")
    public static let homeHealthUnitMinutes = string("home.health.unit.minutes")
    
    public static let settingsSectionRAG = string("settings.section.rag")
    public static let settingsRAGEnable = string("settings.rag.enable")
    public static let settingsRAGDocumentCount = string("settings.rag.document_count")
    public static let settingsRAGDimension = string("settings.rag.dimension")
    public static let settingsRAGComingSoon = string("settings.rag.coming_soon")
    
    public static let settingsSectionDataManagement = string("settings.section.data_management")
    public static let settingsExportData = string("settings.export_data")
    public static let settingsExportDescription = string("settings.export.description")
    public static let settingsClearAllData = string("settings.clear_all_data")
    public static let settingsDataManagementComingSoon = string("settings.data_management.coming_soon")

    public static let settingsLogsTitle = string("settings.logs.title")
    public static let settingsLogsReload = string("settings.logs.reload")
    public static let settingsLogsEmptyTitle = string("settings.logs.empty.title")
    public static let settingsLogsEmptyDescription = string("settings.logs.empty.description")
    public static let settingsLogsFooterSizeFormat = string("settings.logs.footer.size_format")

    public static let settingsSectionAPIKeys = string("settings.section.api_keys")
    public static let settingsAPIKeysFooter = string("settings.api_keys.footer")
    public static let settingsBingWebSearchAPIKeyTitle = string("settings.bing_web_search.api_key.title")
    public static let settingsAPIKeyPlaceholder = string("settings.api_key.placeholder")
    public static let settingsVerifyReceiptTitle = string("settings.verify_receipt.title")
    public static let settingsTestVerifyReceipt = string("settings.verify_receipt.test_button")
    public static let settingsVerifyReceiptSuccessFormat = string("settings.verify.success_format")
    public static let settingsVerifyReceiptErrorFormat = string("settings.verify.error_format")
    
    public static let settingsSectionDEBUG = string("settings.section.debug")
    public static let settingsResetOnboarding = string("settings.reset_onboarding")

    // Connection test status
    public static let settingsConnectionTest = string("settings.connection.test")
    public static let settingsConnectionNotTested = string("settings.connection.not_tested")
    public static let settingsConnectionTesting = string("settings.connection.testing")
    public static let settingsConnectionOKModels = string("settings.connection.ok.models")
    public static let settingsConnectionOKChat = string("settings.connection.ok.chat")
    public static let settingsEchoTesting = string("settings.echo.testing")
    public static let settingsEchoOK = string("settings.echo.ok")
    public static let settingsEchoMismatch = string("settings.echo.mismatch")
    public static let settingsConnectionUnauthorized = string("settings.connection.unauthorized")

    // MARK: - Model Management
    public static let modelManagementTitle = string("model.management.title")
    public static let modelManagementSubtitle = string("model.management.subtitle")
    public static let modelManagementPickerAll = string("model.management.picker.all")
    public static let modelManagementPickerLocal = string("model.management.picker.local")
    public static let modelManagementPickerCloud = string("model.management.picker.cloud")

    public static let modelManagementAdded = string("model.management.added")

    // Local section
    public static let modelManagementLocalTitle = string("model.management.local.title")
    public static let modelManagementLocalOfflineNote = string("model.management.local.offline_note")
    public static let modelManagementDownloadModel = string("model.management.download_model")
    public static let modelManagementDownloaded = string("model.management.downloaded")

    // Cloud section
    public static let modelManagementCloudTitle = string("model.management.cloud.title")
    public static let modelManagementCloudNote = string("model.management.cloud.note")
    public static let modelManagementAddCloudModel = string("model.management.cloud.add")
    public static let modelManagementConfigured = string("model.management.configured")

    // Card / actions
    public static let modelManagementTestModel = string("model.management.test_model")
    public static let modelManagementSaveModel = string("model.management.save_model")
    public static let modelManagementDeleteTitle = string("model.management.delete.title")
    public static let modelManagementTesting = string("model.management.testing")
    public static let modelManagementTestConnection = string("model.management.test_connection")
    public static let modelManagementConnectionSuccess = string("model.management.connection_success")


    public static let modelManagementDeleteSuccessFormat = string("model.management.delete.success_format")

    public static let modelManagementLocalPrivacy = string("model.management.local.privacy")
    public static let modelManagementCloudFeatures = string("model.management.cloud.features")

    public static let modelManagementNoLocalModels = string("model.management.no_local_models")
    public static let modelManagementLocalDownloadHint = string("model.management.local.download_hint")
    public static let modelManagementNoCloudModels = string("model.management.no_cloud_models")
    public static let modelManagementCloudSetupHint = string("model.management.cloud.setup_hint")

    
    // Model Parameters (for ModelParametersView)
    public static let modelParametersContext = string("model.parameters.context")
    public static let modelParametersSampling = string("model.parameters.sampling")
    public static let modelParametersMaxLength = string("model.parameters.max_length")
    public static let modelParametersTopK = string("model.parameters.top_k")
    public static let modelParametersTopP = string("model.parameters.top_p")
    public static let modelParametersTemperature = string("model.parameters.temperature")
    public static let modelParametersSave = string("model.parameters.save")
    public static let modelParametersLengthLabel = string("model.parameters.length_label")
    public static let modelParametersTokensFormat = string("model.parameters.tokens_format")
    public static let modelParametersShowThinking = string("model.parameters.show_thinking")
    public static let modelParametersStreamOutput = string("model.parameters.stream_output")
    public static let modelParametersResetDefaults = string("model.parameters.reset_defaults")
    public static let modelParametersNavigationTitleFormat = string("model.parameters.navigation_title_format")

    // Form / placeholders
    public static let modelManagementFormInfo = string("model.management.form.info")
    public static let modelManagementFormNamePlaceholder = string("model.management.form.name_placeholder")
    public static let modelManagementFormBaseURL = string("model.management.form.base_url")
    public static let modelManagementFormAPIKey = string("model.management.form.api_key")
    public static let modelManagementFormIDPlaceholder = string("model.management.form.id_placeholder")
    public static let modelManagementSourceLocal = string("model.management.source.local")
    public static let modelManagementSourceCloud = string("model.management.source.cloud")

    // Cloud model setup
    public static let cloudModelSetupTitleAdd = string("cloud_model_setup.title.add")
    public static let cloudModelSetupTitleEdit = string("cloud_model_setup.title.edit")

    // Cloud model setup hints
    public static let modelManagementFormHintTitle = string("model.management.form.hint.title")
    public static let modelManagementFormHintURL = string("model.management.form.hint.url")
    public static let modelManagementFormHintID = string("model.management.form.hint.id")
    public static let modelManagementFormHintAPIKey = string("model.management.form.hint.api_key")

    // MARK: - Settings extra keys
    public static let settingsManageURLSources = string("settings.manage.url_sources")
    public static let settingsManagePromptTemplates = string("settings.manage.prompt_templates")
    public static let settingsHealthDataSource = string("settings.health.data_source")
    public static let settingsHealthDataSourceNote = string("settings.health.data_source_note")
    public static let settingsViewPendingNotifications = string("settings.view.pending_notifications")
    public static let settingsClearAllDataWarning = string("settings.clear_all_data.warning")
    public static let settingsExportFailed = string("settings.export.failed")
    public static let settingsClearAllDataDone = string("settings.clear_all_data.done")
    
    public static let settingsLocalLLMDailyLimitTitle = string("settings.local_llm.daily_limit.title")
    public static let settingsDailyTimes = string("settings.daily_times")
    public static let settingsUnlimited = string("settings.unlimited")
    public static let settingsURLDataSourceLimitTitle = string("settings.url_data_source.limit_title")
    public static let settingsManagePromptTemplatesKey = string("settings.manage.prompt_templates")
    public static let settingsCustomPromptDescription = string("settings.custom_prompt.description")
    public static let settingsTemplatesCount = string("settings.templates.count")
    public static let settingsSectionDateRange = string("settings.section.date_range")
    public static let settingsRange7Days = string("settings.range.7days")
    public static let settingsRange30Days = string("settings.range.30days")
    public static let settingsRangeHalfYear = string("settings.range.half_year")
    public static let settingsRangeOneYear = string("settings.range.one_year")
    public static let settingsCalendarRangeDescription = string("settings.calendar.range.description")

    public static let settingsSectionNotifications = string("settings.section.notifications")
    public static let settingsLocalDataSize = string("settings.local_data_size")
    public static let settingsNotificationDaily = string("settings.notification.daily")
    public static let settingsNotificationWeekly = string("settings.notification.weekly")
    public static let settingsNotificationImmediate = string("settings.notification.immediate")
    public static let settingsSilentPeriodTitle = string("settings.silent_period.title")
    public static let settingsStartTime = string("settings.start_time")
    public static let settingsEndTime = string("settings.end_time")
    public static let settingsViewPendingNotificationsKey = string("settings.view.pending_notifications")

    public static let settingsBirthdayLeadTitle = string("settings.birthday_lead.title")
    public static let settingsReminderLeadTitle = string("settings.reminder_lead.title")
    public static let settingsLeadControlDescription = string("settings.lead_control.description")
    public static let settingsEnableNotifications = string("settings.enable_notifications")
    public static let settingsEnableStepReminders = string("settings.enable_step_reminders")
    public static let settingsEnableStepRemindersNote = string("settings.enable_step_reminders.note")
    public static let settingsCalendarAggregateNotificationsTitle = string("settings.calendar_notifications.aggregate.title")
    public static let settingsCalendarAggregateNotificationsDescription = string("settings.calendar_notifications.aggregate.description")
    public static let settingsLocalOnlyActive = string("settings.local_only_active")
    
    // MARK: - Errors
    public static let errorNetworkNotConnected = string("error.network.not_connected")
    public static let errorNetworkTimeout = string("error.network.timeout")
    public static let errorNetworkUnauthorized = string("error.network.unauthorized")
    public static let errorNetworkRateLimited = string("error.network.rate_limited")
    
    public static let errorLLMNotConfigured = string("error.llm.not_configured")
    public static let errorLLMDecoding = string("error.llm.decoding")
    
    public static let errorPermissionDenied = string("error.permission.denied")
    public static let errorNoData = string("error.no_data")

    // Local storage / conversations
    public static let conversationStoreConversationNotFoundFormat = string("conversation_store.error.conversation_not_found_format")

    // Model download
    public static let modelDownloaderDownloadCancelled = string("model_downloader.error.download_cancelled")
    public static let modelDownloaderNoDownloadedFileFound = string("model_downloader.error.no_downloaded_file_found")
    public static let modelDownloaderUnableToOpenFile = string("model_downloader.error.unable_to_open_file")


    // Unified storage
    public static let unifiedStorageInvalidCandidateIndex = string("unified_storage.error.invalid_candidate_index")

    // Health
    public static let healthDataSourceDeallocated = string("health_data_source.error.deallocated")

    // Notifications
    public static let notificationOrchestratorNotificationNotFound = string("notification_orchestrator.error.notification_not_found")
    
    // MARK: - Alerts
    public static let alertTitle = string("alert.title")
    public static let alertPermissionNeeded = string("alert.permission_needed")
    public static let alertPermissionMessage = string("alert.permission_message")

    // MARK: - Notifications
    public static let notificationsTitle = string("notifications.title")
    public static let notificationsNoPending = string("notifications.none_pending")
    public static let notificationsAllSentOrCancelled = string("notifications.all_sent_or_cancelled")
    public static let notificationsFilterTitle = string("notifications.filter.title")
    public static let notificationsFilterAll = string("notifications.filter.all")
    public static let notificationsFilterUnread = string("notifications.filter.unread")
    public static let notificationsSearchPlaceholder = string("notifications.search.placeholder")
    public static let notificationsMarkAllRead = string("notifications.mark_all_read")
    public static let notificationsLoadMore = string("notifications.load_more")
    public static let notificationsMarkRead = string("notifications.mark_read")
    public static let notificationsRead = string("notifications.read")

    // MARK: - Medication
    public static let medicationTitle = string("medication.title")
    public static let medicationCommonDose = string("medication.common_dose")
    public static let medicationNote = string("medication.note")
    public static let medicationSearchPlaceholder = string("medication.search.placeholder")
    public static let medicationNotFound = string("medication.not_found")
    public static let medicationServiceNotInitialized = string("medication.service_not_initialized")
    public static let medicationSearchFailed = string("medication.search_failed")

    // MARK: - Tasks / Parsing
    public static let taskSectionTask = string("task.section.task")
    public static let taskSectionWhen = string("task.section.when")
    public static let taskSectionExtra = string("task.section.extra")
    public static let taskConfirm = string("task.confirm")
    public static let taskIgnore = string("task.ignore")
    public static let errorParseFailed = string("error.parse_failed")
    
    // Task placeholders / labels
    public static let taskTitlePlaceholder = string("task.title.placeholder")
    public static let taskNotesPlaceholder = string("task.notes.placeholder")
    public static let taskToggleRecurring = string("task.toggle.recurring")
    public static let taskDate = string("task.date")
    public static let taskDateOptional = string("task.date.optional")
    public static let taskRecurrenceRule = string("task.recurrence.rule")
    public static let taskRelatedPersonPlaceholder = string("task.related_person.placeholder")
    public static let taskAmountPlaceholder = string("task.amount.placeholder")
    public static let taskSaveFailed = string("task.save_failed")

    // Task inline confirmation
    public static let taskInlinePlannedTime = string("task.inline.planned_time")
    public static let taskInlineNoTimeDetected = string("task.inline.no_time_detected")
    public static let taskInlineSetReminderTime = string("task.inline.set_reminder_time")
    public static let taskInlineSelectTime = string("task.inline.select_time")

    // MARK: - Model Test / Inference
    public static let modelInferenceTest = string("model.inference.test")
    public static let modelLocal = string("model.local")
    public static let modelCloud = string("model.cloud")
    public static let modelPreset = string("model.preset")
    public static let modelGenerating = string("model.generating")
    public static let modelBullet = string("model.bullet")
    public static let modelItemID = string("model.item.id")
    public static let modelExplainTitle = string("model.explain.title")
    public static let modelFillPromptHint = string("model.fill_prompt_hint")
    public static let modelActionGenerate = string("model.action.generate")
    public static let modelActionStop = string("model.action.stop")
    // Model test / UI
    public static let modelTestTitle = string("model.test.title")
    public static let modelTestLocalLabel = string("model.test.local_label")
    public static let modelTestCloudLabel = string("model.test.cloud_label")
    public static let modelTestModelSelect = string("model.test.model_select")
    public static let modelTestMaxTokens = string("model.test.max_tokens")
    public static let modelTest4K = string("model.test.4k")
    public static let modelTest32K = string("model.test.32k")
    public static let modelTestPromptLabel = string("model.test.prompt_label")
    public static let modelTestPreset = string("model.test.preset")
    public static let modelTestTaskLabel = string("model.test.task_label")
    public static let modelTestResultTitle = string("model.test.result_title")
    public static let modelTestGenerating = string("model.test.generating")
    public static let modelTestFillPromptAndRun = string("model.test.fill_prompt_and_run")
    public static let modelTestStop = string("model.test.stop")
    public static let modelTestRun = string("model.test.run")
    public static let modelTestRunning = string("model.test.running")
    public static let modelTestExplain = string("model.test.explain")

    public static let modelTestExecutionEnvironment = string("model.test.execution_environment")

    // Model test detail view
    public static let modelTestDetailParametersTitle = string("model.test.detail.parameters_title")
    public static let modelTestDetailQuickPresets = string("model.test.detail.quick_presets")
    public static let modelTestDetailFailedTitle = string("model.test.detail.failed_title")
    public static let modelTestDetailModelInfo = string("model.test.detail.model_info")
    public static let modelTestDetailModelName = string("model.test.detail.model_name")
    public static let modelTestDetailModelType = string("model.test.detail.model_type")
    public static let modelTestDetailTemperaturePrecise = string("model.test.detail.temperature.precise")
    public static let modelTestDetailTemperatureCreative = string("model.test.detail.temperature.creative")
    public static let modelTestDetailDefaultPrompt1 = string("model.test.detail.default_prompt.1")
    public static let modelTestDetailDefaultPrompt2 = string("model.test.detail.default_prompt.2")
    public static let modelTestDetailDefaultPrompt3 = string("model.test.detail.default_prompt.3")
    // MLX / Local inference specific strings
    public static let modelDownloadedStatus = string("model.downloaded.status")
    public static let modelNotDownloaded = string("model.not_downloaded")
    public static let modelTestMaxTokensValue = string("model.test.max_tokens.value")
    public static let modelTestTemperatureValue = string("model.test.temperature.value")
    public static let modelTestSampleInstruction = string("model.test.sample_instruction")
    public static let mlxTestParamsTitle = string("mlx.test.params_title")
    public static let mlxTestLogsTitle = string("mlx.test.logs_title")
    public static let modelRunLocal = string("model.run.local")
    public static let modelTestGeneratedItems = string("model.test.generated_items")
    public static let modelPriorityFormat = string("model.priority.format")
    public static let modelInferenceTime = string("model.inference.time")
    // Model download view specific
    public static let modelDownloadSectionSources = string("model.download.section.sources")
    public static let modelDownloadSelectSource = string("model.download.select_source")
    public static let modelSourceHuggingFace = string("model.source.huggingface")
    public static let modelSourceModelScope = string("model.source.modelscope")
    public static let modelDownloadSectionAvailable = string("model.download.section.available")
    public static let modelDownloadSectionAll = string("model.download.section.all")
    public static let modelActionUse = string("model.action.use")
    public static let modelActionDownload = string("model.action.download")
    public static let modelLocalTestTitle = string("model.local_test.title")
    public static let modelLocalTestDesc = string("model.local_test.description")
    public static let modelAdvancedTestTitle = string("model.advanced_test.title")
    public static let modelAdvancedTestDesc = string("model.advanced_test.description")
    public static let modelDiskSpaceTitle = string("model.disk_space.title")
    public static let modelDiskDownloadedSize = string("model.disk.downloaded_size")
    public static let modelDeleteAllModels = string("model.delete_all.title")
    public static let modelNotesTitle = string("model.notes.title")
    public static let modelNotesDownloadWifi = string("model.notes.download_wifi")
    public static let modelNotesSaved = string("model.notes.saved")
    public static let modelNotesDeleteSpace = string("model.notes.delete_space")
    public static let modelNotesSourceSuggestion = string("model.notes.source_suggestion")

    // Model status strings
    public static let modelStatusNotDownloaded = string("model.status.not_downloaded")
    public static let modelStatusDownloadFailed = string("model.status.failed")
    public static let modelStatusPaused = string("model.status.paused")
    // Added: human-friendly statuses
    public static let modelStatusReady = string("model.status.ready")
    public static let modelStatusConfigured = string("model.status.configured")
    public static let modelDownloadedCount = string("model.downloaded.count")

    // Model source origin format
    public static let modelSourceOriginFormat = string("model.source.origin_format")

    // MARK: - Small helpers and new keys
    public static let commonBack = string("common.back")

    // URL sources usage format
    public static let urlSourcesUsageFormat = string("urlsources.usage_format")

    // Model delete confirmations
    public static let modelDeleteConfirmSingle = string("model.delete.confirm_single")
    public static let modelDeleteConfirmAll = string("model.delete.confirm_all")

    // Model download progress / size / speed formats
    public static let modelDownloadProgressFormat = string("model.download.progress_format")
    public static let modelDownloadByteProgressFormat = string("model.download.byte_progress_format")
    public static let modelDownloadSpeedFormat = string("model.download.speed_format")
    public static let modelDownloadSizeFormat = string("model.download.size_format")
    
    // Model delete alert buttons/titles
    public static let modelDeleteTitle = string("model.delete.title")
    public static let modelDeleteCancel = string("model.delete.cancel")
    public static let modelDeleteConfirm = string("model.delete.confirm")

    // Model download availability messages
    public static let modelDownloadAvailableOffline = string("model.download.available_offline")
    public static let modelDownloadIncomplete = string("model.download.incomplete")

    // Main tab / Home view strings
    public static let modelNotSelected = string("model.not_selected")
    public static let dataSourcesTitle = string("data_sources.title")
    public static let homeUpdatedAt = string("home.updated_at")
    public static let homeRefresh = string("home.refresh")
    public static let homeGeneratingInProgress = string("home.generating_in_progress")
    public static let homeSummaryTitle = string("home.summary.title")
    public static let homeSummaryItemsCount = string("home.summary.items.count")
    public static let homeNoTodosTitle = string("home.no_todos.title")
    public static let homeNoTodosMessage = string("home.no_todos.message")
    public static let homeDashboardURLSourcesTitle = string("home.dashboard.url_sources.title")
    public static let homeDashboardTodayEventsTitle = string("home.dashboard.today_events.title")
    public static let homeDashboardPartialSourcesFailedTitle = string("home.dashboard.partial_sources_failed.title")
    public static let homeDashboardNoEvents = string("home.dashboard.no_events")
    public static let homeDashboardChipSummaryFormat = string("home.dashboard.chip.summary_format")
    public static let homeDashboardChipEventsFormat = string("home.dashboard.chip.events_format")
    public static let homeDashboardChipURLSourcesFormat = string("home.dashboard.chip.url_sources_format")
    public static let itemActionable = string("item.actionable")
    public static let itemTimeLabel = string("item.time.label")
    public static let notificationViewDetails = string("notification.view.details")
    public static let homeSectionCustomDataSources = string("home.section.custom_data_sources")
    public static let homeFeatureURLTitle = string("home.feature.urls.title")
    public static let homeFeatureURLCount = string("home.feature.urls.count")
    public static let homeFeatureDrugTitle = string("home.feature.drugs.title")
    public static let homeFeatureDrugSubtitle = string("home.feature.drugs.subtitle")
    public static let homeSectionAIConfig = string("home.section.ai_config")
    public static let homeFeaturePromptTemplatesTitle = string("home.feature.prompt_templates.title")
    public static let homeFeatureHealthTitle = string("home.feature.health.title")
    public static let homeFeatureHealthSubtitle = string("home.feature.health.subtitle")
    public static let homeModeCurrent = string("home.mode.current")
    public static let homeModeLocalLabel = string("home.mode.local.label")
    public static let homeModeCloudLabel = string("home.mode.cloud.label")
    public static let homeModeLocalDescription = string("home.mode.local.description")
    public static let homeDownloadedLabel = string("home.downloaded.label")
    public static let homeDownloadedModelsCount = string("home.downloaded.models.count")
    public static let homeManageLocalModels = string("home.manage.local_models")
    public static let homeManageLocalModelsSubtitle = string("home.manage.local_models.subtitle")
    public static let homeDownloadInfo = string("home.download.info")
    public static let apiConfiguration = string("home.api.configuration")
    public static let homeAdvancedTests = string("home.advanced_tests.title")
    public static let homeAdvancedTestsSubtitle = string("home.advanced_tests.subtitle")
    public static let homeStartTest = string("home.start_test.title")
    public static let homeStartTestSubtitle = string("home.start_test.subtitle")
    public static let homeAdvancedTestsDesc = string("home.advanced_tests.desc")
    public static let homeTestingInProgress = string("home.testing_in_progress")
    public static let scheduledActionsTitle = string("scheduled_actions.title")
    public static let scheduledActionsChatHintTitle = string("scheduled_actions.chat_hint.title")
    public static let scheduledActionsChatHintBody = string("scheduled_actions.chat_hint.body")
    public static let scheduledActionsChatHintOpenChat = string("scheduled_actions.chat_hint.open_chat")
    public static let scheduledActionsModeLocal = string("scheduled_actions.mode.local")
    public static let scheduledActionsSchedulerUnavailable = string("scheduled_actions.scheduler.unavailable")
    public static let scheduledActionsWeekdayFormat = string("scheduled_actions.weekday_format")
    public static let scheduledActionsLatestRunSuccess = string("scheduled_actions.latest_run.success")
    public static let scheduledActionsLatestRunPartial = string("scheduled_actions.latest_run.partial")
    public static let scheduledActionsLatestRunSkipped = string("scheduled_actions.latest_run.skipped")
    public static let scheduledActionsLatestRunFailed = string("scheduled_actions.latest_run.failed")
    public static let scheduledActionsSectionOverview = string("scheduled_actions.section.overview")
    public static let scheduledActionsSectionConfigure = string("scheduled_actions.section.configure")
    public static let scheduledActionsSectionHistory = string("scheduled_actions.section.history")
    public static let scheduledActionsOverviewSubtitle = string("scheduled_actions.overview.subtitle")
    public static let scheduledActionsOverviewConfigured = string("scheduled_actions.overview.configured")
    public static let scheduledActionsOverviewDueNow = string("scheduled_actions.overview.due_now")
    public static let scheduledActionsOverviewLastRun = string("scheduled_actions.overview.last_run")
    public static let scheduledActionsOverviewStatusOk = string("scheduled_actions.overview.status.ok")
    public static let scheduledActionsOverviewStatusPartial = string("scheduled_actions.overview.status.partial")
    public static let scheduledActionsOverviewStatusSkipped = string("scheduled_actions.overview.status.skipped")
    public static let scheduledActionsOverviewStatusFailed = string("scheduled_actions.overview.status.failed")
    public static let scheduledActionsRunNow = string("scheduled_actions.run_now")
    public static let scheduledActionsRunning = string("scheduled_actions.running")
    public static let scheduledActionsRunNowNoActions = string("scheduled_actions.run_now.no_actions")
    public static let scheduledActionsRunNowCompleted = string("scheduled_actions.run_now.completed")
    public static let scheduledActionsRunNowCompletedMessage = string("scheduled_actions.run_now.completed.message")
    public static let scheduledActionsRunNowResultFailedTitle = string("scheduled_actions.run_now.result.failed.title")
    public static let scheduledActionsRunNowResultFailedMessage = string("scheduled_actions.run_now.result.failed.message")
    public static let scheduledActionsRunNowResultPartialMessage = string("scheduled_actions.run_now.result.partial.message")
    public static let scheduledActionsRunNowResultSkippedMessage = string("scheduled_actions.run_now.result.skipped.message")
    public static let scheduledActionRunsTitle = string("scheduled_actions.runs.title")
    public static let scheduledActionRunsEmpty = string("scheduled_actions.runs.empty")

    // Scheduled actions - simplified UI
    public static let scheduledActionsOptions = string("scheduled_actions.options")
    public static let scheduledActionsAdvancedOptionsTitle = string("scheduled_actions.advanced_options.title")
    public static let scheduledActionsAdvancedManageTitle = string("scheduled_actions.advanced.manage.title")
    public static let scheduledActionsAdvancedManageSubtitle = string("scheduled_actions.advanced.manage.subtitle")
    public static let scheduledActionsAdvancedEdit = string("scheduled_actions.advanced.edit")
    public static let scheduledActionsDelete = string("scheduled_actions.delete")
    public static let scheduledActionsDeleteConfirmFormat = string("scheduled_actions.delete.confirm_format")
    public static let scheduledActionsScheduleNotSet = string("scheduled_actions.schedule.not_set")

    // Scheduled actions - run detail
    public static let scheduledActionRunDetailTitle = string("scheduled_actions.run_detail.title")
    public static let scheduledActionRunDetailSectionStatus = string("scheduled_actions.run_detail.section.status")
    public static let scheduledActionRunDetailSectionTime = string("scheduled_actions.run_detail.section.time")
    public static let scheduledActionRunDetailSectionAction = string("scheduled_actions.run_detail.section.action")
    public static let scheduledActionRunDetailSectionResult = string("scheduled_actions.run_detail.section.result")
    public static let scheduledActionRunDetailSectionRetry = string("scheduled_actions.run_detail.section.retry")
    public static let scheduledActionRunDetailSectionReason = string("scheduled_actions.run_detail.section.reason")
    public static let scheduledActionRunDetailSectionDetails = string("scheduled_actions.run_detail.section.details")
    public static let scheduledActionRunDetailSectionError = string("scheduled_actions.run_detail.section.error")
    public static let scheduledActionRunDetailSectionNotifications = string("scheduled_actions.run_detail.section.notifications")
    public static let scheduledActionRunDetailSectionModel = string("scheduled_actions.run_detail.section.model")

    public static let scheduledActionRunDetailLabelResult = string("scheduled_actions.run_detail.label.result")
    public static let scheduledActionRunDetailLabelType = string("scheduled_actions.run_detail.label.type")
    public static let scheduledActionRunDetailLabelBackground = string("scheduled_actions.run_detail.label.background")
    public static let scheduledActionRunDetailLabelStart = string("scheduled_actions.run_detail.label.start")
    public static let scheduledActionRunDetailLabelEnd = string("scheduled_actions.run_detail.label.end")
    public static let scheduledActionRunDetailLabelDuration = string("scheduled_actions.run_detail.label.duration")
    public static let scheduledActionRunDetailLabelNextEligible = string("scheduled_actions.run_detail.label.next_eligible")
    public static let scheduledActionRunDetailLabelID = string("scheduled_actions.run_detail.label.id")
    public static let scheduledActionRunDetailLabelName = string("scheduled_actions.run_detail.label.name")
    public static let scheduledActionRunDetailLabelEventsCount = string("scheduled_actions.run_detail.label.events_count")
    public static let scheduledActionRunDetailViewAnalysis = string("scheduled_actions.run_detail.view_analysis")
    public static let scheduledActionRunDetailLabelRetryAllowed = string("scheduled_actions.run_detail.label.retry_allowed")
    public static let scheduledActionRunDetailLabelRetryCount = string("scheduled_actions.run_detail.label.retry_count")
    public static let scheduledActionRunDetailLabelNotificationSent = string("scheduled_actions.run_detail.label.notification_sent")
    public static let scheduledActionRunDetailLabelEventID = string("scheduled_actions.run_detail.label.event_id")
    public static let scheduledActionRunDetailLabelModel = string("scheduled_actions.run_detail.label.model")
    public static let scheduledActionRunDetailLabelTokens = string("scheduled_actions.run_detail.label.tokens")
    public static let scheduledActionRunDetailLabelProcessingDuration = string("scheduled_actions.run_detail.label.processing_duration")

    // Scheduled actions - run reasons
    public static let scheduledActionRunReasonQuietHours = string("scheduled_actions.run_reason.quiet_hours")
    public static let scheduledActionRunReasonNoEvents = string("scheduled_actions.run_reason.no_events")
    public static let scheduledActionRunReasonDeduped = string("scheduled_actions.run_reason.deduped")
    public static let scheduledActionRunReasonFetchFailed = string("scheduled_actions.run_reason.fetch_failed")
    public static let scheduledActionRunReasonFetchTooLarge = string("scheduled_actions.run_reason.fetch_too_large")
    public static let scheduledActionRunReasonFetchBlocked = string("scheduled_actions.run_reason.fetch_blocked")
    public static let scheduledActionRunReasonExecutionFailed = string("scheduled_actions.run_reason.execution_failed")

    // Scheduled actions - journal sheet
    public static let scheduledActionJournalTitle = string("scheduled_actions.journal.title")
    public static let scheduledActionJournalDone = string("scheduled_actions.journal.done")
    public static let scheduledActionJournalStorageUnavailable = string("scheduled_actions.journal.storage_unavailable")
    // Model status helper
    public static let modelStatusDownloading = string("model.status.downloading")
    public static let modelStatusAvailableOffline = string("model.status.available_offline")
    public static let modelStatusNeedDownload = string("model.status.need_download")
    // Generic connection failure
    public static let settingsConnectionFailed = string("settings.connection.failed")
    public static let historyDetailsTitle = string("history.details.title")
    public static let historyDataSourceTitle = string("history.data_source.title")
    public static let historyAISummaryTitle = string("history.ai_summary.title")
    public static let historyProcessingInfoTitle = string("history.processing_info.title")
    public static let historyTokenCountLabel = string("history.token_count.label")
    public static let historyProcessingDurationLabel = string("history.processing_duration.label")
    public static let historyModelLabel = string("history.model.label")
    public static let historyOriginalContentTitle = string("history.original_content.title")
    public static let historyRawResponseTitle = string("history.raw_response.title")
    public static let homeSectionTools = string("home.section.tools")
    // Chat / Conversation strings
    public static let chatNoModelConfigured = string("chat.no_model_configured")
    public static let chatAIThinking = string("chat.ai_thinking")
    public static let chatRetrievalInProgress = string("chat.retrieval_in_progress")

    public static let chatThinkingShort = string("chat.thinking.short")
    public static let chatThinkingProcess = string("chat.thinking.process")
    public static let chatEmptyTitle = string("chat.empty.title")
    public static let chatEmptySubtitle = string("chat.empty.subtitle")
    public static let chatInputPlaceholder = string("chat.input.placeholder")
    public static let chatSend = string("chat.send")
    public static let chatClearTitle = string("chat.clear.title")
    public static let chatClearMessage = string("chat.clear.message")
    public static let chatClearAction = string("chat.clear.action")
    public static let chatClearedTitle = string("chat.cleared.title")
    public static let chatClearedMessage = string("chat.cleared.message")
    public static let chatUndoAction = string("chat.undo.action")
    public static let chatNewConversationPrompt = string("chat.new_conversation_prompt")
    public static let chatNewConversationMessage = string("chat.new_conversation_message")
    // Chat state descriptions
    public static let chatStateIdle = string("chat.state.idle")
    public static let chatStateLoadingLocal = string("chat.state.loading_local")
    public static let chatStateProcessingLocal = string("chat.state.processing_local")
    public static let chatStateProcessingCloud = string("chat.state.processing_cloud")
    public static let chatStateStreamingFormat = string("chat.state.streaming_format")
    public static let chatStateErrorFormat = string("chat.state.error_format")
    public static let chatStateCompleted = string("chat.state.completed")
    // Alerts / buttons
    public static let modelNoAvailableTitle = string("model.no_available.title")
    public static let modelGoDownload = string("model.go_download")
    public static let chatSelectModelPrompt = string("chat.select_model_prompt")
    
    // Notification detail view
    public static let notificationLoading = string("notification.loading")
    public static let notificationDetailTitle = string("notification.detail.title")
    public static let notificationPendingTitle = string("notification.pending.title")
    public static let notificationCompleteItem = string("notification.complete_item")
    public static let notificationScheduledNotice = string("notification.scheduled.notice")
    public static let notificationMarkHandled = string("notification.mark.handled")
    public static let notificationCancelConfirm = string("notification.cancel.confirm")
    public static let notificationOpenRaw = string("notification.open.raw")
    public static let notificationViewInBrowser = string("notification.view_in_browser")
    public static let notificationDetails = string("notification.details")
    public static let notificationContentUnavailable = string("notification.content.unavailable")
    public static let notificationContentRemoved = string("notification.content.removed")
    public static let notificationMarkedHandled = string("notification.marked.handled")
    public static let notificationCancelled = string("notification.cancelled")
    public static let notificationUndo = string("notification.undo")
    public static let notificationPriority = string("notification.priority")
    public static let notificationPriorityFormat = string("notification.priority.format")
    public static let notificationAISummaryTitle = string("notification.ai_summary.title")
    public static let notificationAISummaryDesc = string("notification.ai_summary.desc")
    public static let notificationSuggestedActions = string("notification.suggested_actions")
    public static let notificationTapSummaryAction = string("notification.tap_summary_action")
    public static let notificationAboutSummaryTitle = string("notification.about_summary.title")
    public static let notificationAboutSummaryDesc = string("notification.about_summary.desc")
    public static let notificationAIExplainTitle = string("notification.ai_explain.title")
    public static let notificationRelatedEvent = string("notification.related_event")
    public static let notificationViewDetail = string("notification.view_detail")
    public static let notificationSummaryDetailTitle = string("notification.summary_detail.title")
    // Timeline / misc
    public static let timelineAddedTime = string("timeline.added_time")
    public static let timelineScheduledNotification = string("timeline.scheduled_notification")
    public static let notificationConfirmMarkTitle = string("notification.confirm_mark.title")
    public static let notificationNotesTitle = string("notification.notes.title")
    // Prompt templates
    public static let promptTemplatesCurrent = string("prompt.current")
    public static let promptTemplatesBuiltIn = string("prompt.builtin")
    public static let promptTemplatesCustom = string("prompt.custom")
    public static let promptTemplatesVariables = string("prompt.variables")
    public static let promptTemplatesUse = string("prompt.use")
    public static let promptTemplatesView = string("prompt.view")
    public static let promptTemplatesSaveAs = string("prompt.save_as_template")
    public static let promptTemplatesEstimateTokens = string("prompt.estimate.tokens")
    public static let promptTemplatesContentTitle = string("prompt.content.title")
    public static let promptTemplatesNewTitle = string("prompt.template.new_title")
    public static let promptTemplatesEditTitle = string("prompt.template.edit_title")
    public static let promptTemplatesWarning = string("prompt.warning")

    // Common copy/delete/save
    public static let commonCopy = string("common.copy")
    public static let commonDelete = string("common.delete")
    public static let commonSave = string("common.save")
    public static let commonName = string("common.name")
    public static let commonDescription = string("common.description")
    public static let promptTemplatesBasicInfo = string("prompt.basic.info")
    public static let promptTemplatesTitle = string("prompt.title")
    public static let preview = string("common.preview")
    public static let edit = string("common.edit")
    



    // MARK: - URL Data Sources
    public static let urlSourcesEmptyTitle = string("urlsources.empty.title")
    public static let urlSourcesEmptySubtitle = string("urlsources.empty.subtitle")
    public static let urlSourcesUsageLabel = string("urlsources.usage_label")
    public static let urlSourcesNavigationTitle = string("urlsources.navigation.title")
    public static let urlSourcesDeleteTitle = string("urlsources.delete.title")
    public static let urlSourcesDeleteCancel = string("urlsources.delete.cancel")
    public static let urlSourcesDeleteConfirm = string("urlsources.delete.confirm")
    public static let urlSourcesDeleteMessage = string("urlsources.delete.message")

    public static let urlSourcesDetectedFormat = string("urlsources.detected_format")
    public static let urlSourcesFormatAutoDetect = string("urlsources.format.auto_detect")
    public static let urlSourcesFormatHTML = string("urlsources.format.html")
    public static let urlSourcesFormatRSS = string("urlsources.format.rss")
    public static let urlSourcesFormatJSON = string("urlsources.format.json")
    public static let urlSourcesFormatiCalendar = string("urlsources.format.icalendar")
    public static let urlSourcesDataTypeTitle = string("urlsources.data_type.title")
    public static let urlSourcesDataTypeWillDetect = string("urlsources.data_type.will_detect")
    public static let urlSourcesURLField = string("urlsources.url_field")
    public static let urlSourcesValidationSectionTitle = string("urlsources.section.validation")
    public static let urlSourcesValidationReadyNote = string("urlsources.validation.ready_note")
    public static let urlSourcesValidationFailureNote = string("urlsources.validation.failure_note")
    public static let urlSourcesUserPromptPlaceholderExamples = string("urlsources.user_prompt.placeholder_examples")
    public static let urlSourcesUserPromptHelp = string("urlsources.user_prompt.help")
    public static let urlSourcesModelSelectionTitle = string("urlsources.section.model_selection")
    public static let urlSourcesTestSectionTitle = string("urlsources.section.test")
    public static let urlSourcesTesting = string("urlsources.testing")
    public static let urlSourcesTestNote = string("urlsources.test.note")
    public static let urlSourcesValidating = string("urlsources.validating")
    public static let urlSourcesValidate = string("urlsources.validate")
    public static let urlSourcesValidateSuccess = string("urlsources.validate.success")
    public static let urlSourcesValidateFailure = string("urlsources.validate.failure")
    public static let urlSourcesMaxLimitReached = string("urlsources.max_limit_reached")
    public static let urlSourcesURLEmpty = string("urlsources.url_empty")
    public static let urlSourcesFetcherUnavailable = string("urlsources.fetcher_unavailable")

    public static let urlSourcesTestRun = string("urlsources.test.run")
    public static let urlSourcesPushTestToggle = string("urlsources.test.push_toggle")
    public static let urlSourcesPushTestNote = string("urlsources.test.push_note")
    public static let urlSourcesTestOutputTitle = string("urlsources.test.output_title")
    public static let urlSourcesCopy = string("urlsources.copy")
    public static let urlSourcesEditTitle = string("urlsources.edit.title")
    public static let urlSourcesAddTitle = string("urlsources.add.title")
    public static let urlSourcesSaveLabel = string("urlsources.action.save")
    public static let urlSourcesAddLabel = string("urlsources.action.add")

    // URL source history (MainTabView)
    public static let urlSourceHistoryEmpty = string("url.source.history.empty")

    // MARK: - Background Refresh
    public static let backgroundRefreshSectionTitle = string("background_refresh.section.title")
    public static let backgroundRefreshDisabledMessage = string("background_refresh.disabled.message")
    public static let backgroundRefreshOpenSystemSettings = string("background_refresh.open_settings")
    public static let backgroundRefreshHint = string("background_refresh.hint")
    public static let backgroundRefreshLowPowerModeNote = string("background_refresh.low_power_mode_note")


    public static let modelPickerTitle = string("model.picker.title")

    // Models view
    public static let modelsTitle = string("models.title")
    public static let modelsSectionLocal = string("models.section.local")
    public static let modelsSectionCloudAPI = string("models.section.cloud_api")
    public static let modelsCloudCustomAPITitle = string("models.cloud.custom_api.title")
    public static let modelsCloudCustomAPIDescription = string("models.cloud.custom_api.description")
    public static let modelsStatusDownloading = string("models.status.downloading")
    public static let modelsStatusReady = string("models.status.ready")
    public static let modelsStatusNotDownloaded = string("models.status.not_downloaded")


    // Relative times
    public static let relativeJustNow = string("relative.just_now")
    public static let relativeMinutesAgo = string("relative.minutes_ago")
    public static let relativeHoursAgo = string("relative.hours_ago")
    public static let relativeDaysAgo = string("relative.days_ago")
}

// MARK: - String Extension for Easy Localization

extension String {
    /// Localized version of the string.
    var localized: String {
        NSLocalizedString(self, comment: "")
    }
    
    /// Localized with arguments.
    func localized(_ args: CVarArg...) -> String {
        String(format: NSLocalizedString(self, comment: ""), arguments: args)
    }
}
