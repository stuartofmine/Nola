use std::env;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    ZhHans,
    En,
    De,
    Es,
    Fr,
    Ja,
    Ru,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum Message {
    ErrorPrefix,
    NoSavedSessions,
    MessagesUnit,
    SessionLabel,
    CreatedLabel,
    UpdatedLabel,
    MessagesLabel,
    DeletedSessionPrefix,
    FailedReadPromptPrefix,
    PromptRequired,
    SessionRequiresAction,
    RequiresSessionIdSuffix,
    UnknownSessionActionPrefix,
    UnknownCommandPrefix,
    MaxIterationsPositive,
    UnknownOptionPrefix,
    UnexpectedSessionArgs,
    MissingEndpoint,
    ContinueOrSession,
    CurrentDirFailedPrefix,
    RequiresValueSuffix,
    FailedResolvePathPrefix,
    ActiveSessionLabel,
    FailedCreateSessionDirPrefix,
    FailedReadSessionPrefix,
    FailedParseSessionPrefix,
    FailedSerializeSessionPrefix,
    FailedSaveSessionPrefix,
    FailedReadSessionsPrefix,
    FailedReadSessionEntryPrefix,
    FailedDeleteSessionPrefix,
    HomeNotSet,
    FailedReadLatestSessionPrefix,
    FailedSaveLatestSessionPrefix,
    InvalidSessionId,
    FailedCreateHttpClientPrefix,
    LlmRequestFailedPrefix,
    LlmStreamReadFailedPrefix,
    ToolRequiresApprovalPrefix,
    ToolRequiresApprovalSuffix,
    ArgsLabel,
    AllowOncePrompt,
    PermissionDeniedByUser,
    PermissionDeniedGuidance,
    SubAgentLabel,
    TaskLabel,
    CapabilityLabel,
    PromptLabel,
    DelegateAgentCapability,
    DelegateAgentConfirmationRequired,
    NetworkAskNonInteractive,
    NetworkAskPrompt,
    NetworkAllowedByUser,
    NetworkDeniedByUser,
    DisplayReadPrefix,
    DisplayWrotePrefix,
    DisplayEditedPrefix,
    DisplayListedPrefix,
    EntriesUnit,
    MatchesUnit,
    ReachedMaxIterationsPrefix,
    ReachedMaxIterationsSuffix,
    InteractiveQueuedRequest,
    InteractiveRunning,
    InteractiveReadyForNextRequest,
    InteractiveModelLabel,
    InteractiveParamsLabel,
    InteractiveWorkingDirLabel,
    InteractiveWorkspaceLabel,
    InteractiveProviderLabel,
    InteractiveProviderLocal,
    InteractiveModeFieldLabel,
    InteractiveNetworkFieldLabel,
    InteractiveEndpointLabel,
    InteractiveAvailableModels,
    InteractiveModelUsageHint,
    InteractiveAvailableCommands,
    InteractiveSlashHelpHint,
    InteractiveSlashModelHint,
    InteractiveSlashExecutorHint,
    InteractiveAvailableExecutors,
    InteractiveExecutorUsage,
    InteractiveExecutorSelected,
    InteractiveExecutorUnavailable,
    InteractiveExecutorLabel,
    InteractiveSlashDebugHint,
    InteractiveSlashDebugUsage,
    InteractiveSlashAllowHint,
    InteractiveSlashAllowUsage,
    InteractiveAllowPanelTitle,
    InteractiveAllowPanelCautious,
    InteractiveAllowPanelCautiousDesc,
    InteractiveAllowPanelAllowAll,
    InteractiveAllowPanelAllowAllDesc,
    InteractiveAllowPanelSelected,
    InteractiveAllowPanelCancelled,
    InteractiveSlashGoalHint,
    InteractiveSlashGoalUsage,
    InteractiveSlashInitHint,
    InteractiveSlashCdHint,
    InteractiveSlashCompactHint,
    InteractiveSlashNewHint,
    InteractiveSlashPlanHint,
    InteractiveSlashJobsHint,
    InteractiveSlashResumeHint,
    InteractiveSlashContinueHint,
    InteractiveSlashHistoryHint,
    InteractiveSlashStatusHint,
    InteractiveSlashExitHint,
    InteractiveSlashQuitHint,
    InteractiveWelcomeTagline,
    InteractiveWelcomeHint,
    InteractiveAutoValue,
    InteractiveNetworkAllowValue,
    InteractiveNetworkAskValue,
    InteractiveNetworkDenyValue,
    InteractiveContextLabel,
    InteractiveTokensLabel,
    InteractiveTokensUnit,
    InteractiveMessagesUnit,
    InteractiveAllowPanelErrorPrefix,
    InteractiveGoalStatusPrefix,
    InteractiveGoalStatePrefix,
    InteractiveGoalStateRunning,
    InteractiveGoalStatePaused,
    InteractiveGoalStateNeedsUser,
    InteractiveGoalStateFailed,
    InteractiveGoalResumeStarted,
    InteractiveGoalNoResumable,
    InteractiveWorkspaceStatusPrefix,
    InteractiveCdUsage,
    InteractiveCompactTitle,
    InteractiveCompactNoActiveSession,
    InteractiveCompactAlready,
    InteractiveCompactCompleted,
    InteractiveCompactSummarized,
    InteractiveCompactKeptRecent,
    InteractiveCompactSaved,
    InteractiveCompactHistoryPreserved,
    InteractiveCompactFailedPrefix,
    InteractiveNewSessionTitle,
    InteractiveNewSessionPending,
    InteractiveJobsTitle,
    InteractiveJobsUsage,
    InteractiveUnknownCommandHint,
    InteractiveChoiceLabel,
    InteractiveModelStatusPrefix,
    InteractiveDebugStatusPrefix,
    InteractiveAllowStatusPrefix,
    InteractiveDefaultModelLabel,
    InteractiveTurnsLabel,
    InteractiveToolsLabel,
    InteractiveNetworkLabel,
    InteractiveUnknownPath,
    InteractiveStatusOn,
    InteractiveStatusOff,
    InteractiveInputPlaceholder,
    InteractiveExecutionDraftPrefix,
    InteractiveExecutionQueuedPrefix,
    InteractiveExecutionInterruptHint,
    InteractiveExecutionInputHint,
    AgentStatusThinking,
    AgentStatusRetryingModel,
    AgentStatusStrictToolStrategy,
    AgentStatusChooseFromAvailableTools,
    AgentStatusReadyToContinue,
    AgentStatusEnteredMode,
    AgentStatusReplyingDirectly,
    AgentStatusExecutingPlan,
    AgentStatusWaitingForUserConfirmation,
    AgentStatusProcessing,
    AgentStatusPreparing,
    AgentStatusModelUnavailable,
    AgentDoneLabel,
    AgentErrorLabel,
    AgentToolLabel,
    AgentAgentLabel,
    AgentSessionLabel,
    AgentStatusLabel,
    AgentInfoLabel,
    AgentReadyCurrentModePrefix,
    AgentReadyCurrentModeSuffix,
    AgentStatusDetailsSeparator,
    AgentPreparingToExecutePrefix,
    AgentActionCountSuffixSingle,
    AgentActionCountSuffixPlural,
    AgentTurnPrefix,
    AgentTurnSuffix,
    AgentInspectingProject,
    AgentEditingFiles,
    AgentRunningCommand,
    AgentAccessingNetwork,
    AgentSearchingTools,
    AgentStartingSubtask,
    AgentFileModified,
    AgentCommandCompleted,
    AgentNetworkRequestCompleted,
    AgentSubtaskCompleted,
    AgentFailedToModifyFile,
    AgentCommandFailed,
    AgentNetworkRequestFailed,
    AgentSubtaskFailed,
    AgentOperationFailed,
    InteractiveInvalidLanguageValue,
    InvalidPortValue,
    UnknownAgentModePrefix,
    NetworkPolicyMustBeOneOf,
    ForceToolSetMustBeOneOf,
    FailedReadKeyEventPrefix,
    FailedFlushPromptPrefix,
    InteractiveRawModeFailedPrefix,
}

static LANGUAGE_OVERRIDE: OnceLock<Language> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
pub struct PermissionText {
    pub allow_once_option: &'static str,
    pub deny_option: &'static str,
    pub keyboard_hint: &'static str,
    pub once_only_keyboard_hint: &'static str,
    pub session_unavailable_option: &'static str,
    pub custom_response: &'static str,
    pub allow_conversation_prefix: &'static str,
    pub file_access_title: &'static str,
    pub default_working_directory: &'static str,
    pub cwd: &'static str,
    pub command: &'static str,
    pub reason: &'static str,
    pub endpoint: &'static str,
    pub model: &'static str,
    pub path: &'static str,
    pub directory: &'static str,
    pub allowed: &'static str,
    pub tool: &'static str,
    pub summary: &'static str,
}

pub fn permission_text() -> PermissionText {
    match current_language() {
        Language::ZhHans => PermissionText {
            allow_once_option: "仅允许本次",
            deny_option: "拒绝",
            keyboard_hint: "使用 ↑/↓ 选择，Enter 确认 · 也可按 1/2/3/N",
            once_only_keyboard_hint: "使用 ↑/↓ 选择，Enter 确认 · 也可按 1/3/N",
            session_unavailable_option: "本会话允许（当前请求未提供此选项）",
            custom_response: "返回主输入框…",
            allow_conversation_prefix: "本会话允许：",
            file_access_title: "文件访问需要授权",
            default_working_directory: "默认工作目录",
            cwd: "工作目录",
            command: "命令",
            reason: "原因",
            endpoint: "端点",
            model: "模型",
            path: "路径",
            directory: "目录",
            allowed: "已允许",
            tool: "工具",
            summary: "摘要",
        },
        Language::De => PermissionText {
            allow_once_option: "Einmal erlauben",
            deny_option: "Ablehnen",
            keyboard_hint: "Mit ↑/↓ wählen, mit Enter bestätigen · auch 1/2/3/N",
            once_only_keyboard_hint: "Mit ↑/↓ wählen, mit Enter bestätigen · auch 1/3/N",
            session_unavailable_option: "Für diese Sitzung erlauben (für diese Anfrage nicht angeboten)",
            custom_response: "Zur Haupteingabe zurück…",
            allow_conversation_prefix: "Für diese Sitzung erlauben:",
            file_access_title: "Dateizugriff erfordert Genehmigung",
            default_working_directory: "Standard-Arbeitsverzeichnis",
            cwd: "Arbeitsordner",
            command: "Befehl",
            reason: "Grund",
            endpoint: "Endpunkt",
            model: "Modell",
            path: "Pfad",
            directory: "Verzeichnis",
            allowed: "Erlaubt",
            tool: "Werkzeug",
            summary: "Zusammenfassung",
        },
        Language::Es => PermissionText {
            allow_once_option: "Permitir una vez",
            deny_option: "Denegar",
            keyboard_hint: "Usa ↑/↓ y Enter para confirmar · también 1/2/3/N",
            once_only_keyboard_hint: "Usa ↑/↓ y Enter para confirmar · también 1/3/N",
            session_unavailable_option: "Permitir durante esta sesión (no disponible para esta solicitud)",
            custom_response: "Volver a la entrada…",
            allow_conversation_prefix: "Permitir durante esta sesión:",
            file_access_title: "El acceso a archivos requiere permiso",
            default_working_directory: "Directorio de trabajo predeterminado",
            cwd: "Directorio",
            command: "Comando",
            reason: "Motivo",
            endpoint: "Endpoint",
            model: "Modelo",
            path: "Ruta",
            directory: "Directorio",
            allowed: "Permitido",
            tool: "Herramienta",
            summary: "Resumen",
        },
        Language::Fr => PermissionText {
            allow_once_option: "Autoriser une fois",
            deny_option: "Refuser",
            keyboard_hint: "Utilisez ↑/↓ puis Entrée · 1/2/3/N fonctionnent aussi",
            once_only_keyboard_hint: "Utilisez ↑/↓ puis Entrée · 1/3/N fonctionnent aussi",
            session_unavailable_option: "Autoriser pour cette session (option non proposée pour cette demande)",
            custom_response: "Revenir à la saisie…",
            allow_conversation_prefix: "Autoriser pour cette session :",
            file_access_title: "L’accès aux fichiers nécessite une autorisation",
            default_working_directory: "Répertoire de travail par défaut",
            cwd: "Répertoire",
            command: "Commande",
            reason: "Motif",
            endpoint: "Point d’accès",
            model: "Modèle",
            path: "Chemin",
            directory: "Répertoire",
            allowed: "Autorisé",
            tool: "Outil",
            summary: "Résumé",
        },
        Language::Ja => PermissionText {
            allow_once_option: "今回だけ許可",
            deny_option: "拒否",
            keyboard_hint: "↑/↓ で選択し Enter で確定 · 1/2/3/N も使用可能",
            once_only_keyboard_hint: "↑/↓ で選択し Enter で確定 · 1/3/N も使用可能",
            session_unavailable_option: "このセッションで許可（このリクエストでは選択できません）",
            custom_response: "メイン入力欄に戻る…",
            allow_conversation_prefix: "このセッションで許可：",
            file_access_title: "ファイルアクセスには許可が必要です",
            default_working_directory: "既定の作業ディレクトリ",
            cwd: "作業場所",
            command: "コマンド",
            reason: "理由",
            endpoint: "エンドポイント",
            model: "モデル",
            path: "パス",
            directory: "ディレクトリ",
            allowed: "許可済み",
            tool: "ツール",
            summary: "概要",
        },
        Language::Ru => PermissionText {
            allow_once_option: "Разрешить один раз",
            deny_option: "Отклонить",
            keyboard_hint: "Выберите ↑/↓ и подтвердите Enter · также 1/2/3/N",
            once_only_keyboard_hint: "Выберите ↑/↓ и подтвердите Enter · также 1/3/N",
            session_unavailable_option: "Разрешить для этого сеанса (не предлагается для этого запроса)",
            custom_response: "Вернуться к вводу…",
            allow_conversation_prefix: "Разрешить для этого сеанса:",
            file_access_title: "Для доступа к файлам требуется разрешение",
            default_working_directory: "Рабочий каталог по умолчанию",
            cwd: "Рабочий каталог",
            command: "Команда",
            reason: "Причина",
            endpoint: "Эндпоинт",
            model: "Модель",
            path: "Путь",
            directory: "Каталог",
            allowed: "Разрешено",
            tool: "Инструмент",
            summary: "Сводка",
        },
        Language::En => PermissionText {
            allow_once_option: "Allow once",
            deny_option: "Deny",
            keyboard_hint: "Use ↑/↓, Enter to confirm · 1/2/3/N also work",
            once_only_keyboard_hint: "Use ↑/↓, Enter to confirm · 1/3/N also work",
            session_unavailable_option: "Allow for this session (not offered for this request)",
            custom_response: "Return to composer…",
            allow_conversation_prefix: "Allow for this session:",
            file_access_title: "File access requires approval",
            default_working_directory: "default working directory",
            cwd: "cwd",
            command: "command",
            reason: "reason",
            endpoint: "endpoint",
            model: "model",
            path: "path",
            directory: "directory",
            allowed: "allowed",
            tool: "tool",
            summary: "summary",
        },
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InteractionText {
    pub selection_hint: &'static str,
    pub current_label: &'static str,
    pub question_input_prefix: &'static str,
    pub question_or_direct_answer: &'static str,
    pub question_invalid_range_prefix: &'static str,
    pub question_invalid: &'static str,
    pub custom_answer: &'static str,
    pub resume_title: &'static str,
    pub resume_inline_unsupported: &'static str,
    pub resume_json_unsupported: &'static str,
    pub resume_terminal_only: &'static str,
    pub session_list: &'static str,
    pub recent_sessions_hint: &'static str,
    pub resume_input_prompt: &'static str,
    pub resume_invalid_prefix: &'static str,
    pub resume_invalid_suffix: &'static str,
    pub empty_value: &'static str,
    pub no_execution_plan: &'static str,
    pub execution_plan: &'static str,
    pub completed: &'static str,
    pub complete: &'static str,
    pub in_progress: &'static str,
    pub pending: &'static str,
    pub current_project_session: &'static str,
    pub loading_session_history: &'static str,
    pub messages_suffix: &'static str,
    pub no_matches: &'static str,
    pub no_matches_in_window: &'static str,
    pub confirmation_required: &'static str,
    pub decision_required: &'static str,
    pub question_cancelled: &'static str,
    pub question_input_required: &'static str,
    pub answers_received_suffix: &'static str,
    pub validation_pipeline_rejected: &'static str,
    pub custom_permission_return: &'static str,
}

pub fn interaction_text() -> InteractionText {
    interaction_text_for(current_language())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiText {
    HelpSources,
    DataSourcesTitle,
    DataSourcesUsage,
    DataSourceAdded,
    DataSourceTested,
    DataSourceRemoved,
    DataSourceEnabled,
    DataSourceDisabled,
    DataSourceInherited,
    DataSourceNoSession,
    DataSourceEmpty,
    DataSourceUnknownAction,
    DataSourcePolicy,
    DataSourceRecords,
    DataSourceVerified,
    PromptCache,
    CacheRead,
    CacheWrite,
    CacheSections,
    CacheDisabled,
    CacheInvalidIdentity,
    CacheFirstRequest,
    CacheHit,
    CacheHitRate,
    CacheStored,
    CacheMiss,
    CacheStableCandidate,
    CacheUnknown,
    Answer,
    Session,
    SessionBusy,
    WaitInOriginalTerminal,
    Plan,
    AllComplete,
    WaitingNextStep,
    Background,
    TasksUnit,
    RunningUnit,
    ManageJobs,
    MoreTasks,
    Agents,
    AgentUsage,
    Tasks,
    TasksUsage,
    ResumeUsage,
    ContinueUsage,
    AutonomousGoal,
    Constraints,
    DefaultSafeConstraints,
    Mode,
    AutonomousUntilComplete,
    NoAvailableModels,
    HelpAgent,
    Work,
    WorkUsage,
    WorkResolveHint,
    HelpWork,
    NoWork,
    WorkActive,
    WorkFooter,
    WorkId,
    WorkKind,
    WorkParent,
    WorkCurrentStep,
    WorkSopVersion,
    WorkSopUpdateAvailable,
    WorkSopPinnedHint,
    WorkRerunStarted,
    WorkRerunRequiresTerminal,
    WorkSopDefinitionUnavailable,
    WorkNotFound,
    WorkControlUnsupported,
    WorkStopped,
    WorkPaused,
    WorkResumed,
    WorkGoal,
    WorkSop,
    WorkSkill,
    WorkTask,
    WorkProcess,
    WorkExecutor,
    WorkQueued,
    InputQueueFull,
    InputAdmissionFailed,
    WorkPlanning,
    WorkReady,
    WorkWaitingUser,
    WorkWaitingExternal,
    WorkVerifying,
    WorkCompensating,
    WorkArtifacts,
    WorkEvidence,
    HelpTasks,
    HelpJobs,
    GoalPaused,
    ConfigNotFound,
    CreateConfigPrompt,
    EndpointPrompt,
    EndpointRequired,
    ModelPrompt,
    ApiKeyPrompt,
    ConfigCreated,
    InitCheckOutput,
    InitScanFiles,
    InitAnalyzeProject,
    InitGenerateMemory,
    InitWriteNola,
    NoBackgroundTasks,
    BackgroundTasksTitle,
    TasksFooter,
    Task,
    Status,
    Agent,
    Progress,
    Result,
    Error,
    RecentActivity,
    Queued,
    Running,
    WaitingPermission,
    Completed,
    Failed,
    Cancelled,
    ExitCode,
    Using,
    SubagentPermissionRequest,
    AllowResourceForTask,
    AllowedTaskOnce,
    AllowedConversationSession,
    SentCustomTaskResponse,
    DeniedTaskRequest,
    Goal,
    Items,
    Risks,
    Verify,
    ExecutionPlan,
    Sources,
    Page,
    Lines,
}

pub fn ui_text(key: UiText) -> &'static str {
    ui_text_for(current_language(), key)
}

fn ui_text_for(language: Language, key: UiText) -> &'static str {
    let values = match key {
        UiText::HelpSources => ["管理数据源：/sources [list|add|test|remove|enable|disable|inherit]", "Manage data sources: /sources [list|add|test|remove|enable|disable|inherit]", "Datenquellen verwalten: /sources [list|add|test|remove|enable|disable|inherit]", "Gestionar fuentes de datos: /sources [list|add|test|remove|enable|disable|inherit]", "Gérer les sources de données : /sources [list|add|test|remove|enable|disable|inherit]", "データソースを管理: /sources [list|add|test|remove|enable|disable|inherit]", "Управление источниками данных: /sources [list|add|test|remove|enable|disable|inherit]"],
        UiText::DataSourcesTitle => ["数据源", "Data sources", "Datenquellen", "Fuentes de datos", "Sources de données", "データソース", "Источники данных"],
        UiText::DataSourcesUsage => ["用法", "Usage", "Verwendung", "Uso", "Utilisation", "使い方", "Использование"],
        UiText::DataSourceAdded => ["数据源已测试并添加", "Data source tested and added", "Datenquelle getestet und hinzugefügt", "Fuente de datos probada y añadida", "Source de données testée et ajoutée", "データソースをテストして追加しました", "Источник данных проверен и добавлен"],
        UiText::DataSourceTested => ["数据源测试成功", "Data source test passed", "Datenquellentest erfolgreich", "Prueba de fuente de datos superada", "Test de la source de données réussi", "データソースのテストに成功しました", "Проверка источника данных пройдена"],
        UiText::DataSourceRemoved => ["数据源已删除", "Data source removed", "Datenquelle entfernt", "Fuente de datos eliminada", "Source de données supprimée", "データソースを削除しました", "Источник данных удалён"],
        UiText::DataSourceEnabled => ["已在当前会话启用", "Enabled for this conversation", "Für diese Unterhaltung aktiviert", "Activada para esta conversación", "Activée pour cette conversation", "この会話で有効にしました", "Включено для этого диалога"],
        UiText::DataSourceDisabled => ["已在当前会话停用", "Disabled for this conversation", "Für diese Unterhaltung deaktiviert", "Desactivada para esta conversación", "Désactivée pour cette conversation", "この会話で無効にしました", "Отключено для этого диалога"],
        UiText::DataSourceInherited => ["已恢复默认策略", "Default policy restored", "Standardrichtlinie wiederhergestellt", "Política predeterminada restaurada", "Politique par défaut restaurée", "既定のポリシーに戻しました", "Политика по умолчанию восстановлена"],
        UiText::DataSourceNoSession => ["当前没有可用会话", "No active conversation", "Keine aktive Unterhaltung", "No hay una conversación activa", "Aucune conversation active", "アクティブな会話がありません", "Нет активного диалога"],
        UiText::DataSourceEmpty => ["尚未添加数据源", "No data sources yet", "Noch keine Datenquellen", "Aún no hay fuentes de datos", "Aucune source de données", "データソースはまだありません", "Источники данных ещё не добавлены"],
        UiText::DataSourceUnknownAction => ["未知的数据源操作", "Unknown data-source action", "Unbekannte Datenquellenaktion", "Acción de fuente de datos desconocida", "Action de source de données inconnue", "不明なデータソース操作です", "Неизвестное действие с источником данных"],
        UiText::DataSourcePolicy => ["策略", "Policy", "Richtlinie", "Política", "Politique", "ポリシー", "Политика"],
        UiText::DataSourceRecords => ["条使用记录", "usage records", "Nutzungsprotokolle", "registros de uso", "enregistrements d’utilisation", "件の利用記録", "записей использования"],
        UiText::DataSourceVerified => ["已验证", "Verified", "Verifiziert", "Verificada", "Vérifiée", "検証済み", "Проверено"],
        UiText::PromptCache => ["提示缓存", "Prompt cache", "Prompt-Cache", "Caché de instrucciones", "Cache d’invite", "プロンプトキャッシュ", "Кэш промпта"],
        UiText::CacheRead => ["读取", "read", "gelesen", "leída", "lecture", "読み取り", "чтение"],
        UiText::CacheWrite => ["写入", "write", "geschrieben", "escrita", "écriture", "書き込み", "запись"],
        UiText::CacheSections => ["片段变化", "section changes", "Abschnittsänderungen", "cambios de sección", "changements de section", "セクション変更", "изменения разделов"],
        UiText::CacheDisabled => ["已关闭", "disabled", "deaktiviert", "deshabilitada", "désactivé", "無効", "отключено"],
        UiText::CacheInvalidIdentity => ["标识无效", "invalid identity", "ungültige Identität", "identidad no válida", "identité non valide", "識別情報が無効", "недопустимый идентификатор"],
        UiText::CacheFirstRequest => ["首次请求", "first request", "erste Anfrage", "primera solicitud", "première requête", "初回リクエスト", "первый запрос"],
        UiText::CacheHit => ["命中", "hit", "Treffer", "acierto", "succès", "ヒット", "попадание"],
        UiText::CacheHitRate => ["缓存命中率", "Cache hit rate", "Cache-Trefferrate", "Aciertos de caché", "Taux de cache", "キャッシュ命中率", "Попадания в кэш"],
        UiText::CacheStored => ["已写入", "stored", "gespeichert", "guardada", "enregistré", "保存済み", "сохранено"],
        UiText::CacheMiss => ["未命中", "miss", "nicht getroffen", "fallo", "échec", "ミス", "промах"],
        UiText::CacheStableCandidate => ["稳定候选", "stable candidate", "stabiler Kandidat", "candidata estable", "candidat stable", "安定候補", "стабильный кандидат"],
        UiText::CacheUnknown => ["状态未知", "unknown", "unbekannt", "desconocido", "inconnu", "不明", "неизвестно"],
        UiText::Sources => ["来源", "Sources", "Quellen", "Fuentes", "Sources", "出典", "Источники"],
        UiText::Page => ["页", "page", "Seite", "página", "page", "ページ", "страница"],
        UiText::Lines => ["行", "lines", "Zeilen", "líneas", "lignes", "行", "строки"],
        UiText::Answer => ["回答", "Answer", "Antwort", "Respuesta", "Réponse", "回答", "Ответ"],
        UiText::Session => ["会话", "Session", "Sitzung", "Sesión", "Session", "セッション", "Сеанс"],
        UiText::SessionBusy => ["正在执行", "Running", "Wird ausgeführt", "En ejecución", "En cours", "実行中", "Выполняется"],
        UiText::WaitInOriginalTerminal => ["请在原终端中断或等待完成", "Wait or interrupt it in the original terminal", "Im ursprünglichen Terminal warten oder abbrechen", "Espera o interrumpe en el terminal original", "Attendez ou interrompez dans le terminal d’origine", "元のターミナルで待機または中断してください", "Дождитесь завершения или прервите в исходном терминале"],
        UiText::Plan => ["计划", "Plan", "Plan", "Plan", "Plan", "計画", "План"],
        UiText::AllComplete => ["全部完成", "All complete", "Alles abgeschlossen", "Todo completado", "Tout est terminé", "すべて完了", "Всё завершено"],
        UiText::WaitingNextStep => ["等待下一步", "Waiting for next step", "Warten auf den nächsten Schritt", "Esperando el siguiente paso", "En attente de l’étape suivante", "次のステップを待機中", "Ожидание следующего шага"],
        UiText::Background => ["后台", "Background", "Hintergrund", "Segundo plano", "Arrière-plan", "バックグラウンド", "Фон"],
        UiText::TasksUnit => ["个任务", "tasks", "Aufgaben", "tareas", "tâches", "件のタスク", "задач"],
        UiText::RunningUnit => ["个运行中", "running", "aktiv", "en ejecución", "en cours", "件を実行中", "выполняется"],
        UiText::ManageJobs => ["用 /jobs 管理", "manage with /jobs", "mit /jobs verwalten", "gestionar con /jobs", "gérer avec /jobs", "/jobs で管理", "управление через /jobs"],
        UiText::MoreTasks => ["其余任务请用 /jobs 查看", "more tasks; use /jobs", "weitere Aufgaben; /jobs verwenden", "tareas más; usa /jobs", "tâches supplémentaires ; utilisez /jobs", "件の残りは /jobs で確認", "других задач; используйте /jobs"],
        UiText::Agents => ["子代理", "Agents", "Agenten", "Agentes", "Agents", "サブエージェント", "Агенты"],
        UiText::AgentUsage => ["用法：/agent <类型> <任务>", "Usage: /agent <type> <task>", "Verwendung: /agent <Typ> <Aufgabe>", "Uso: /agent <tipo> <tarea>", "Utilisation : /agent <type> <tâche>", "使い方: /agent <種類> <タスク>", "Использование: /agent <тип> <задача>"],
        UiText::Tasks => ["后台任务", "Tasks", "Aufgaben", "Tareas", "Tâches", "バックグラウンドタスク", "Задачи"],
        UiText::TasksUsage => ["用法：/tasks [<任务ID> | stop <任务ID>]", "Usage: /tasks [<task-id> | stop <task-id>]", "Verwendung: /tasks [<Aufgaben-ID> | stop <Aufgaben-ID>]", "Uso: /tasks [<id-tarea> | stop <id-tarea>]", "Utilisation : /tasks [<id-tâche> | stop <id-tâche>]", "使い方: /tasks [<タスクID> | stop <タスクID>]", "Использование: /tasks [<ID-задачи> | stop <ID-задачи>]"],
        UiText::ResumeUsage => ["用法：/resume", "Usage: /resume", "Verwendung: /resume", "Uso: /resume", "Utilisation : /resume", "使い方: /resume", "Использование: /resume"],
        UiText::ContinueUsage => ["用法：/continue", "Usage: /continue", "Verwendung: /continue", "Uso: /continue", "Utilisation : /continue", "使い方: /continue", "Использование: /continue"],
        UiText::AutonomousGoal => ["自主目标", "Autonomous goal", "Autonomes Ziel", "Objetivo autónomo", "Objectif autonome", "自律目標", "Автономная цель"],
        UiText::Constraints => ["约束", "Constraints", "Einschränkungen", "Restricciones", "Contraintes", "制約", "Ограничения"],
        UiText::DefaultSafeConstraints => ["使用默认安全约束", "Using default safety constraints", "Standard-Sicherheitsregeln werden verwendet", "Se usan las restricciones de seguridad predeterminadas", "Contraintes de sécurité par défaut utilisées", "既定の安全制約を使用", "Используются стандартные ограничения безопасности"],
        UiText::Mode => ["模式", "Mode", "Modus", "Modo", "Mode", "モード", "Режим"],
        UiText::AutonomousUntilComplete => ["持续推进，完成后自动报告", "Continue autonomously until complete", "Autonom bis zum Abschluss fortfahren", "Continuar de forma autónoma hasta terminar", "Continuer de façon autonome jusqu’à la fin", "完了まで自律的に継続", "Продолжать автономно до завершения"],
        UiText::NoAvailableModels => ["没有可用模型", "No available models", "Keine Modelle verfügbar", "No hay modelos disponibles", "Aucun modèle disponible", "利用可能なモデルがありません", "Нет доступных моделей"],
        UiText::HelpAgent => ["启动或列出可用的子代理", "Start or list available sub-agents", "Unteragenten starten oder auflisten", "Iniciar o listar subagentes", "Démarrer ou lister les sous-agents", "サブエージェントを開始または一覧表示", "Запустить или показать субагентов"],
        UiText::Work => ["工作", "Work", "Arbeit", "Trabajo", "Travail", "作業", "Работа"],
        UiText::WorkUsage => ["用法：/work [<工作ID> | status|stop|pause|resume|rerun <工作ID>]", "Usage: /work [<work-id> | status|stop|pause|resume|rerun <work-id>]", "Verwendung: /work [<Arbeits-ID> | status|stop|pause|resume|rerun <Arbeits-ID>]", "Uso: /work [<id-trabajo> | status|stop|pause|resume|rerun <id-trabajo>]", "Utilisation : /work [<id-travail> | status|stop|pause|resume|rerun <id-travail>]", "使い方: /work [<作業ID> | status|stop|pause|resume|rerun <作業ID>]", "Использование: /work [<ID-работы> | status|stop|pause|resume|rerun <ID-работы>]"],
        UiText::WorkResolveHint => ["外部结果仍未知。保留原记录；只能通过精确执行恢复控制核验回执、授权风险重试或保持受阻。", "The external outcome is unknown. Preserve the record; use exact execution recovery to verify a receipt, authorize a risky retry, or remain blocked.", "Das externe Ergebnis ist unbekannt. Den Datensatz behalten; eine Wiederherstellung erfordert den genauen Ausführungsnachweis oder eine ausdrückliche Risikoentscheidung.", "El resultado externo es desconocido. Conserva el registro; la recuperación exige el recibo exacto o una decisión explícita sobre el riesgo.", "Le résultat externe est inconnu. Conservez la trace ; la reprise exige un reçu exact ou une décision explicite sur le risque.", "外部結果は不明です。記録を保持し、正確な実行の復旧操作で受領証または明示的なリスク判断を確認してください。", "Внешний результат неизвестен. Сохраните запись; восстановление требует точной квитанции или явного решения о риске."],
        UiText::HelpWork => ["统一查看和控制目标、任务、后台进程与外部执行器", "Inspect and control goals, tasks, background processes, and external executors", "Ziele, Aufgaben, Hintergrundprozesse und externe Ausführer gemeinsam verwalten", "Inspeccionar y controlar objetivos, tareas, procesos en segundo plano y ejecutores externos", "Afficher et contrôler les objectifs, tâches, processus en arrière-plan et exécuteurs externes", "目標、タスク、バックグラウンド処理、外部実行環境をまとめて管理", "Просмотр и управление целями, задачами, фоновыми процессами и внешними исполнителями"],
        UiText::NoWork => ["当前会话没有工作记录。", "No work in this conversation.", "Keine Arbeit in dieser Unterhaltung.", "No hay trabajo en esta conversación.", "Aucun travail dans cette conversation.", "この会話に作業はありません。", "В этом диалоге нет работы."],
        UiText::WorkActive => ["项进行中", "active", "aktiv", "activos", "actifs", "件進行中", "активно"],
        UiText::WorkFooter => ["使用 `/work <工作ID>` 查看详情；使用 `/work stop <工作ID>` 停止。已结束的流程可用 `/work rerun <工作ID>` 按当前定义重新运行。", "Use `/work <work-id>` for details and `/work stop <work-id>` to stop. A finished SOP can be started as a new run with `/work rerun <work-id>`.", "Details mit `/work <Arbeits-ID>`, stoppen mit `/work stop <Arbeits-ID>`. Einen beendeten SOP-Ablauf starten Sie mit `/work rerun <Arbeits-ID>` als neuen Lauf.", "Usa `/work <id-trabajo>` para ver detalles y `/work stop <id-trabajo>` para detener. Un SOP finalizado puede iniciarse como una ejecución nueva con `/work rerun <id-trabajo>`.", "Utilisez `/work <id-travail>` pour les détails et `/work stop <id-travail>` pour arrêter. Un SOP terminé peut être relancé dans une nouvelle exécution avec `/work rerun <id-travail>`.", "`/work <作業ID>` で詳細、`/work stop <作業ID>` で停止します。終了した SOP は `/work rerun <作業ID>` で新しい実行として開始できます。", "Используйте `/work <ID-работы>` для подробностей и `/work stop <ID-работы>` для остановки. Завершённый SOP можно запустить заново командой `/work rerun <ID-работы>`."],
        UiText::WorkId => ["工作 ID", "Work ID", "Arbeits-ID", "ID de trabajo", "ID du travail", "作業 ID", "ID работы"],
        UiText::WorkKind => ["类型", "Kind", "Typ", "Tipo", "Type", "種類", "Тип"],
        UiText::WorkParent => ["上级工作", "Parent work", "Übergeordnete Arbeit", "Trabajo principal", "Travail parent", "親作業", "Родительская работа"],
        UiText::WorkCurrentStep => ["当前步骤", "Current step", "Aktueller Schritt", "Paso actual", "Étape actuelle", "現在のステップ", "Текущий шаг"],
        UiText::WorkSopVersion => ["流程版本", "SOP version", "SOP-Version", "Versión del SOP", "Version du SOP", "SOP バージョン", "Версия SOP"],
        UiText::WorkSopUpdateAvailable => ["有新的流程定义", "Definition update available", "Neue SOP-Definition verfügbar", "Hay una definición nueva disponible", "Nouvelle définition disponible", "新しい定義があります", "Доступно обновление определения"],
        UiText::WorkSopPinnedHint => ["当前运行继续使用已固定的定义；结束后可运行 `/work rerun <工作ID>`，以最新定义创建新运行。", "This run remains pinned to its original definition. After it ends, use `/work rerun <work-id>` to create a new run from the latest definition.", "Dieser Lauf bleibt an seine ursprüngliche Definition gebunden. Danach erstellt `/work rerun <Arbeits-ID>` einen neuen Lauf mit der neuesten Definition.", "Esta ejecución permanece fijada a su definición original. Cuando termine, usa `/work rerun <id-trabajo>` para crear otra con la definición más reciente.", "Cette exécution reste liée à sa définition d’origine. Une fois terminée, utilisez `/work rerun <id-travail>` pour créer une nouvelle exécution avec la dernière définition.", "この実行は開始時の定義に固定されています。終了後に `/work rerun <作業ID>` を使うと最新定義で新しい実行を作成できます。", "Этот запуск остаётся привязан к исходному определению. После завершения используйте `/work rerun <ID-работы>`, чтобы создать новый запуск с актуальным определением."],
        UiText::WorkRerunStarted => ["已创建新的流程运行", "Started a new SOP run", "Neuen SOP-Lauf gestartet", "Se inició una ejecución SOP nueva", "Nouvelle exécution SOP créée", "新しい SOP 実行を開始しました", "Создан новый запуск SOP"],
        UiText::WorkRerunRequiresTerminal => ["请先停止或完成原流程运行，再创建新运行。", "Stop or finish the original SOP run before creating a new run.", "Beenden oder stoppen Sie den ursprünglichen SOP-Lauf, bevor Sie einen neuen erstellen.", "Detén o finaliza la ejecución SOP original antes de crear otra.", "Arrêtez ou terminez l’exécution SOP d’origine avant d’en créer une nouvelle.", "新しい実行を作成する前に、元の SOP 実行を停止または完了してください。", "Перед созданием нового запуска остановите или завершите исходный запуск SOP."],
        UiText::WorkSopDefinitionUnavailable => ["当前客户端没有可用于该流程的新定义。", "No current definition is available for this SOP.", "Für diesen SOP ist keine aktuelle Definition verfügbar.", "No hay una definición actual disponible para este SOP.", "Aucune définition actuelle n’est disponible pour ce SOP.", "この SOP に利用できる現在の定義がありません。", "Для этого SOP нет доступного актуального определения."],
        UiText::WorkNotFound => ["未找到工作", "Work not found", "Arbeit nicht gefunden", "Trabajo no encontrado", "Travail introuvable", "作業が見つかりません", "Работа не найдена"],
        UiText::WorkControlUnsupported => ["该工作类型暂不支持此操作。", "This control is not supported for that work type.", "Diese Aktion wird für diesen Arbeitstyp nicht unterstützt.", "Este control no es compatible con ese tipo de trabajo.", "Cette commande n’est pas prise en charge pour ce type de travail.", "この作業タイプではその操作を利用できません。", "Эта операция не поддерживается для данного типа работы."],
        UiText::WorkStopped => ["已请求停止", "Stop requested", "Stopp angefordert", "Detención solicitada", "Arrêt demandé", "停止を要求しました", "Запрошена остановка"],
        UiText::WorkPaused => ["已暂停", "Paused", "Pausiert", "En pausa", "En pause", "一時停止しました", "Приостановлено"],
        UiText::WorkResumed => ["已继续", "Resumed", "Fortgesetzt", "Reanudado", "Repris", "再開しました", "Возобновлено"],
        UiText::WorkGoal => ["目标", "Goal", "Ziel", "Objetivo", "Objectif", "目標", "Цель"],
        UiText::WorkSop => ["标准流程", "SOP run", "SOP-Ablauf", "Ejecución SOP", "Exécution SOP", "SOP 実行", "Выполнение SOP"],
        UiText::WorkSkill => ["技能运行", "Skill run", "Skill-Ausführung", "Ejecución de habilidad", "Exécution de compétence", "スキル実行", "Запуск навыка"],
        UiText::WorkTask => ["任务", "Task", "Aufgabe", "Tarea", "Tâche", "タスク", "Задача"],
        UiText::WorkProcess => ["后台进程", "Background process", "Hintergrundprozess", "Proceso en segundo plano", "Processus en arrière-plan", "バックグラウンド処理", "Фоновый процесс"],
        UiText::WorkExecutor => ["外部执行器", "External executor", "Externer Ausführer", "Ejecutor externo", "Exécuteur externe", "外部実行環境", "Внешний исполнитель"],
        UiText::WorkQueued => ["排队中", "Queued", "In Warteschlange", "En cola", "En file d’attente", "待機中", "В очереди"],
        UiText::InputQueueFull => ["队列已满，输入已保留", "Queue full; draft retained", "Warteschlange voll; Entwurf behalten", "Cola llena; borrador conservado", "File pleine ; brouillon conservé", "待機列がいっぱいです。下書きは保持されました", "Очередь заполнена; черновик сохранён"],
        UiText::InputAdmissionFailed => ["未确认输入已接收，草稿已保留", "Input admission unconfirmed; draft retained", "Eingangsbestätigung fehlt; Entwurf behalten", "Recepción sin confirmar; borrador conservado", "Réception non confirmée ; brouillon conservé", "入力の受領を確認できません。下書きは保持されました", "Приём ввода не подтверждён; черновик сохранён"],
        UiText::WorkPlanning => ["规划中", "Planning", "Planung", "Planificando", "Planification", "計画中", "Планирование"],
        UiText::WorkReady => ["就绪", "Ready", "Bereit", "Listo", "Prêt", "準備完了", "Готово"],
        UiText::WorkWaitingUser => ["等待用户", "Waiting for user", "Wartet auf Benutzer", "Esperando al usuario", "En attente de l’utilisateur", "ユーザー待ち", "Ожидание пользователя"],
        UiText::WorkWaitingExternal => ["等待外部操作", "Waiting for external work", "Wartet auf externe Arbeit", "Esperando trabajo externo", "En attente d’un travail externe", "外部処理待ち", "Ожидание внешней работы"],
        UiText::WorkVerifying => ["验证中", "Verifying", "Überprüfung", "Verificando", "Vérification", "検証中", "Проверка"],
        UiText::WorkCompensating => ["补偿中", "Compensating", "Kompensation", "Compensando", "Compensation", "補償中", "Компенсация"],
        UiText::WorkArtifacts => ["产物", "Artifacts", "Artefakte", "Artefactos", "Artefacts", "成果物", "Артефакты"],
        UiText::WorkEvidence => ["证据", "Evidence", "Nachweise", "Evidencia", "Preuves", "証拠", "Доказательства"],
        UiText::HelpTasks => ["查看或停止后台任务；权限请求会自动弹出", "Inspect or stop background tasks; permission prompts open automatically", "Hintergrundaufgaben anzeigen oder stoppen; Berechtigungen öffnen automatisch", "Ver o detener tareas en segundo plano; los permisos se abren automáticamente", "Voir ou arrêter les tâches en arrière-plan ; les autorisations s’ouvrent automatiquement", "バックグラウンドタスクの確認・停止。権限画面は自動表示", "Просмотреть или остановить фоновые задачи; запрос разрешения откроется автоматически"],
        UiText::HelpJobs => ["管理后台任务", "Manage background tasks", "Hintergrundaufgaben verwalten", "Gestionar tareas en segundo plano", "Gérer les tâches en arrière-plan", "バックグラウンドタスクを管理", "Управление фоновыми задачами"],
        UiText::GoalPaused => ["目标已暂停", "Goal paused", "Ziel pausiert", "Objetivo en pausa", "Objectif en pause", "目標を一時停止", "Цель приостановлена"],
        UiText::ConfigNotFound => ["未找到 Nola 配置：", "Nola config was not found at", "Nola-Konfiguration nicht gefunden:", "No se encontró la configuración de Nola en", "Configuration Nola introuvable :", "Nola 設定が見つかりません:", "Конфигурация Nola не найдена:"],
        UiText::CreateConfigPrompt => ["现在创建？[Y/n] ", "Create it now? [Y/n] ", "Jetzt erstellen? [Y/n] ", "¿Crearla ahora? [Y/n] ", "La créer maintenant ? [Y/n] ", "今すぐ作成しますか？ [Y/n] ", "Создать сейчас? [Y/n] "],
        UiText::EndpointPrompt => ["LLM 端点（通常以 /v1 结尾）：", "LLM endpoint, usually ending with /v1: ", "LLM-Endpunkt, meist mit /v1 am Ende: ", "Endpoint LLM, normalmente termina en /v1: ", "Endpoint LLM, se terminant généralement par /v1 : ", "LLM エンドポイント（通常 /v1 で終わります）: ", "Эндпоинт LLM, обычно с /v1 в конце: "],
        UiText::EndpointRequired => ["端点不能为空。", "Endpoint is required.", "Endpunkt ist erforderlich.", "El endpoint es obligatorio.", "L’endpoint est requis.", "エンドポイントは必須です。", "Эндпоинт обязателен."],
        UiText::ModelPrompt => ["模型名称（可选）：", "Model name (optional): ", "Modellname (optional): ", "Nombre del modelo (opcional): ", "Nom du modèle (facultatif) : ", "モデル名（任意）: ", "Название модели (необязательно): "],
        UiText::ApiKeyPrompt => ["API Key（可选，本地模型可留空）：", "API key (optional, leave empty for local models): ", "API-Key (optional, für lokale Modelle leer lassen): ", "Clave API (opcional, vacía para modelos locales): ", "Clé API (facultative, vide pour les modèles locaux) : ", "API キー（任意、ローカルモデルは空欄可）: ", "API-ключ (необязательно, для локальных моделей можно оставить пустым): "],
        UiText::ConfigCreated => ["已创建", "Created", "Erstellt", "Creado", "Créé", "作成しました", "Создано"],
        UiText::InitCheckOutput => ["检查输出位置", "Check output location", "Ausgabeort prüfen", "Comprobar ubicación de salida", "Vérifier l’emplacement de sortie", "出力先を確認", "Проверка места вывода"],
        UiText::InitScanFiles => ["扫描项目文件", "Scan project files", "Projektdateien scannen", "Examinar archivos del proyecto", "Analyser les fichiers du projet", "プロジェクトファイルを走査", "Сканирование файлов проекта"],
        UiText::InitAnalyzeProject => ["分析项目结构", "Analyze project structure", "Projektstruktur analysieren", "Analizar estructura del proyecto", "Analyser la structure du projet", "プロジェクト構造を分析", "Анализ структуры проекта"],
        UiText::InitGenerateMemory => ["生成项目记忆", "Generate project memory", "Projektspeicher erzeugen", "Generar memoria del proyecto", "Générer la mémoire du projet", "プロジェクトメモリを生成", "Создание памяти проекта"],
        UiText::InitWriteNola => ["写入 NOLA.md", "Write NOLA.md", "NOLA.md schreiben", "Escribir NOLA.md", "Écrire NOLA.md", "NOLA.md を書き込み", "Запись NOLA.md"],
        UiText::NoBackgroundTasks => ["当前工作区没有后台任务。", "No background tasks in this workspace.", "In diesem Arbeitsbereich gibt es keine Hintergrundaufgaben.", "No hay tareas en segundo plano en este espacio de trabajo.", "Aucune tâche en arrière-plan dans cet espace de travail.", "このワークスペースにバックグラウンドタスクはありません。", "В этой рабочей области нет фоновых задач."],
        UiText::BackgroundTasksTitle => ["# 后台任务", "# Background tasks", "# Hintergrundaufgaben", "# Tareas en segundo plano", "# Tâches en arrière-plan", "# バックグラウンドタスク", "# Фоновые задачи"],
        UiText::TasksFooter => ["使用 `/tasks <任务ID>` 查看结果，`/tasks stop <任务ID>` 停止任务；权限请求会自动打开审批面板。", "Use `/tasks <task-id>` to inspect results and `/tasks stop <task-id>` to stop a task; permission requests open the approval panel automatically.", "Mit `/tasks <Aufgaben-ID>` Ergebnisse anzeigen und mit `/tasks stop <Aufgaben-ID>` stoppen; Berechtigungsanfragen öffnen automatisch den Dialog.", "Usa `/tasks <id-tarea>` para ver resultados y `/tasks stop <id-tarea>` para detenerla; los permisos abren el panel automáticamente.", "Utilisez `/tasks <id-tâche>` pour voir les résultats et `/tasks stop <id-tâche>` pour l’arrêter ; les autorisations ouvrent automatiquement le panneau.", "`/tasks <タスクID>` で結果を確認し、`/tasks stop <タスクID>` で停止します。権限要求は自動的に承認画面を開きます。", "Используйте `/tasks <ID-задачи>` для результатов и `/tasks stop <ID-задачи>` для остановки; запросы разрешений автоматически откроют панель."],
        UiText::Task => ["任务", "Task", "Aufgabe", "Tarea", "Tâche", "タスク", "Задача"],
        UiText::Status => ["状态", "Status", "Status", "Estado", "État", "状態", "Статус"],
        UiText::Agent => ["代理", "Agent", "Agent", "Agente", "Agent", "エージェント", "Агент"],
        UiText::Progress => ["进度", "Progress", "Fortschritt", "Progreso", "Progression", "進捗", "Прогресс"],
        UiText::Result => ["结果", "Result", "Ergebnis", "Resultado", "Résultat", "結果", "Результат"],
        UiText::Error => ["错误", "Error", "Fehler", "Error", "Erreur", "エラー", "Ошибка"],
        UiText::RecentActivity => ["最近活动", "Recent activity", "Letzte Aktivität", "Actividad reciente", "Activité récente", "最近のアクティビティ", "Недавняя активность"],
        UiText::Queued => ["排队中", "Queued", "In Warteschlange", "En cola", "En file d’attente", "待機中", "В очереди"],
        UiText::Running => ["运行中", "Running", "Läuft", "En ejecución", "En cours", "実行中", "Выполняется"],
        UiText::WaitingPermission => ["等待授权", "Waiting for approval", "Wartet auf Freigabe", "Esperando aprobación", "En attente d’autorisation", "承認待ち", "Ожидание разрешения"],
        UiText::Completed => ["已完成", "Completed", "Abgeschlossen", "Completada", "Terminée", "完了", "Завершено"],
        UiText::Failed => ["失败", "Failed", "Fehlgeschlagen", "Fallida", "Échec", "失敗", "Ошибка"],
        UiText::Cancelled => ["已取消", "Cancelled", "Abgebrochen", "Cancelada", "Annulée", "キャンセル済み", "Отменено"],
        UiText::ExitCode => ["退出码", "Exit code", "Exit-Code", "Código de salida", "Code de sortie", "終了コード", "Код выхода"],
        UiText::Using => ["正在使用", "Using", "Verwendet", "Usando", "Utilisation de", "使用中", "Используется"],
        UiText::SubagentPermissionRequest => ["子代理请求执行", "Sub-agent requests", "Unteragent fordert an", "El subagente solicita", "Le sous-agent demande", "サブエージェントが要求", "Субагент запрашивает"],
        UiText::AllowResourceForTask => ["本任务允许此资源", "Allow this resource for this task", "Diese Ressource für diese Aufgabe erlauben", "Permitir este recurso para esta tarea", "Autoriser cette ressource pour cette tâche", "このタスクでこのリソースを許可", "Разрешить этот ресурс для этой задачи"],
        UiText::AllowedTaskOnce => ["已允许任务执行一次", "Allowed the task once", "Aufgabe einmalig zugelassen", "Tarea permitida una vez", "Tâche autorisée une fois", "タスクを一度許可", "Задача разрешена один раз"],
        UiText::AllowedConversationSession => ["已允许本会话执行同类操作", "Allowed similar operations for this session", "Ähnliche Aktionen für diese Sitzung zugelassen", "Acciones similares permitidas para esta sesión", "Actions similaires autorisées pour cette session", "このセッションで同様の操作を許可しました", "Похожие действия разрешены для этого сеанса"],
        UiText::SentCustomTaskResponse => ["已向任务发送自定义回复", "Sent a custom response to the task", "Eigene Antwort an die Aufgabe gesendet", "Respuesta personalizada enviada a la tarea", "Réponse personnalisée envoyée à la tâche", "タスクにカスタム回答を送信", "Пользовательский ответ отправлен задаче"],
        UiText::DeniedTaskRequest => ["已拒绝任务请求", "Denied the task request", "Aufgabenanfrage abgelehnt", "Solicitud de tarea rechazada", "Demande de tâche refusée", "タスク要求を拒否", "Запрос задачи отклонён"],
        UiText::Goal => ["目标", "Goal", "Ziel", "Objetivo", "Objectif", "目標", "Цель"],
        UiText::Items => ["项任务", "items", "Einträge", "elementos", "éléments", "件", "элементов"],
        UiText::Risks => ["注意", "Risks", "Risiken", "Riesgos", "Risques", "注意", "Риски"],
        UiText::Verify => ["验证", "Verify", "Prüfen", "Verificar", "Vérifier", "検証", "Проверка"],
        UiText::ExecutionPlan => ["执行计划", "Execution plan", "Ausführungsplan", "Plan de ejecución", "Plan d’exécution", "実行計画", "План выполнения"],
    };
    values[match language {
        Language::ZhHans => 0,
        Language::En => 1,
        Language::De => 2,
        Language::Es => 3,
        Language::Fr => 4,
        Language::Ja => 5,
        Language::Ru => 6,
    }]
}

fn interaction_text_for(language: Language) -> InteractionText {
    match language {
        Language::ZhHans => InteractionText {
            selection_hint: "↑/↓ 选择 · Enter 确认 · Esc 取消",
            current_label: "当前：",
            question_input_prefix: "  输入",
            question_or_direct_answer: "，或直接输入回答",
            question_invalid_range_prefix: "  请输入有效序号，范围为 1-",
            question_invalid: "  请输入有效序号，或输入 q 取消",
            custom_answer: "自定义回答…",
            resume_title: "恢复会话",
            resume_inline_unsupported: "恢复会话模式不支持行内提示",
            resume_json_unsupported: "恢复会话模式不支持 --json",
            resume_terminal_only: "恢复会话模式仅支持交互式终端",
            session_list: "会话列表",
            recent_sessions_hint: "最近会话 · ↑↓ 选择 · Enter 恢复 · Esc 退出",
            resume_input_prompt: "输入序号并按 Enter（输入 q 退出）：",
            resume_invalid_prefix: "输入无效，请输入 1-",
            resume_invalid_suffix: "，或输入 q 退出",
            empty_value: "<空>",
            no_execution_plan: "当前会话还没有执行计划",
            execution_plan: "执行计划",
            completed: "完成",
            complete: "已完成",
            in_progress: "进行中",
            pending: "待处理",
            current_project_session: "当前项目会话",
            loading_session_history: "加载会话历史",
            messages_suffix: "条消息",
            no_matches: "无匹配",
            no_matches_in_window: "当前窗口无匹配",
            confirmation_required: "需要用户确认",
            decision_required: "需要你的选择",
            question_cancelled: "用户取消了选择",
            question_input_required: "需要用户输入",
            answers_received_suffix: "个回答已收到",
            validation_pipeline_rejected: "验证命令必须直接运行完成，不能通过 head 或 tail 截断输出",
            custom_permission_return: "已返回主输入框，请继续输入",
        },
        Language::De => InteractionText {
            selection_hint: "↑/↓ wählen · Enter bestätigen · Esc abbrechen",
            current_label: "Aktuell:",
            question_input_prefix: "  Eingabe",
            question_or_direct_answer: " oder Antwort direkt eingeben",
            question_invalid_range_prefix: "  Bitte eine gültige Zahl eingeben: 1-",
            question_invalid: "  Gültige Zahl eingeben oder mit q abbrechen",
            custom_answer: "Eigene Antwort…",
            resume_title: "Sitzung fortsetzen",
            resume_inline_unsupported: "Im Fortsetzungsmodus ist keine Inline-Eingabe möglich",
            resume_json_unsupported: "Der Fortsetzungsmodus unterstützt --json nicht",
            resume_terminal_only: "Der Fortsetzungsmodus benötigt ein interaktives Terminal",
            session_list: "Sitzungsliste",
            recent_sessions_hint: "Letzte Sitzungen · ↑↓ wählen · Enter fortsetzen · Esc schließen",
            resume_input_prompt: "Nummer eingeben und Enter drücken (q zum Beenden):",
            resume_invalid_prefix: "Ungültig. Bitte 1-",
            resume_invalid_suffix: " eingeben oder q zum Beenden",
            empty_value: "<leer>",
            no_execution_plan: "Diese Sitzung hat noch keinen Ausführungsplan",
            execution_plan: "Ausführungsplan",
            completed: "abgeschlossen",
            complete: "Fertig",
            in_progress: "Läuft",
            pending: "Ausstehend",
            current_project_session: "Aktuelle Projektsitzung",
            loading_session_history: "Sitzungsverlauf wird geladen",
            messages_suffix: "Nachrichten",
            no_matches: "Keine Treffer",
            no_matches_in_window: "Keine Treffer im aktuellen Bereich",
            confirmation_required: "Bestätigung erforderlich",
            decision_required: "Entscheidung erforderlich",
            question_cancelled: "Auswahl abgebrochen",
            question_input_required: "Benutzereingabe erforderlich",
            answers_received_suffix: "Antworten erhalten",
            validation_pipeline_rejected: "Validierungsbefehle müssen vollständig und ohne head/tail ausgeführt werden",
            custom_permission_return: "Zur Haupteingabe zurückgekehrt; fahren Sie mit der Eingabe fort",
        },
        Language::Es => InteractionText {
            selection_hint: "↑/↓ seleccionar · Enter confirmar · Esc cancelar",
            current_label: "Actual:",
            question_input_prefix: "  Introduce",
            question_or_direct_answer: " o escribe una respuesta",
            question_invalid_range_prefix: "  Introduce un número válido: 1-",
            question_invalid: "  Introduce un número válido o q para cancelar",
            custom_answer: "Respuesta personalizada…",
            resume_title: "Reanudar sesión",
            resume_inline_unsupported: "El modo de reanudación no admite mensajes en línea",
            resume_json_unsupported: "El modo de reanudación no admite --json",
            resume_terminal_only: "El modo de reanudación requiere un terminal interactivo",
            session_list: "Lista de sesiones",
            recent_sessions_hint: "Sesiones recientes · ↑↓ seleccionar · Enter reanudar · Esc salir",
            resume_input_prompt: "Introduce un número y pulsa Enter (q para salir):",
            resume_invalid_prefix: "Entrada no válida. Introduce 1-",
            resume_invalid_suffix: " o q para salir",
            empty_value: "<vacío>",
            no_execution_plan: "Esta sesión aún no tiene un plan de ejecución",
            execution_plan: "Plan de ejecución",
            completed: "completado",
            complete: "Completo",
            in_progress: "En curso",
            pending: "Pendiente",
            current_project_session: "Sesión actual del proyecto",
            loading_session_history: "Cargando historial de la sesión",
            messages_suffix: "mensajes",
            no_matches: "Sin coincidencias",
            no_matches_in_window: "Sin coincidencias en la ventana actual",
            confirmation_required: "Se requiere confirmación",
            decision_required: "Se requiere una decisión",
            question_cancelled: "Selección cancelada",
            question_input_required: "Se requiere la entrada del usuario",
            answers_received_suffix: "respuestas recibidas",
            validation_pipeline_rejected: "Los comandos de validación deben completarse sin truncar la salida con head o tail",
            custom_permission_return: "Has vuelto a la entrada principal; continúa escribiendo",
        },
        Language::Fr => InteractionText {
            selection_hint: "↑/↓ sélectionner · Entrée confirmer · Échap annuler",
            current_label: "Actuel :",
            question_input_prefix: "  Saisissez",
            question_or_direct_answer: " ou saisissez une réponse",
            question_invalid_range_prefix: "  Saisissez un numéro valide : 1-",
            question_invalid: "  Saisissez un numéro valide ou q pour annuler",
            custom_answer: "Réponse personnalisée…",
            resume_title: "Reprendre une session",
            resume_inline_unsupported: "Le mode de reprise n’accepte pas de message en ligne",
            resume_json_unsupported: "Le mode de reprise ne prend pas en charge --json",
            resume_terminal_only: "Le mode de reprise nécessite un terminal interactif",
            session_list: "Liste des sessions",
            recent_sessions_hint: "Sessions récentes · ↑↓ sélectionner · Entrée reprendre · Échap quitter",
            resume_input_prompt: "Saisissez un numéro puis Entrée (q pour quitter) :",
            resume_invalid_prefix: "Entrée invalide. Saisissez 1-",
            resume_invalid_suffix: " ou q pour quitter",
            empty_value: "<vide>",
            no_execution_plan: "Cette session n’a pas encore de plan d’exécution",
            execution_plan: "Plan d’exécution",
            completed: "terminé",
            complete: "Terminé",
            in_progress: "En cours",
            pending: "En attente",
            current_project_session: "Session de projet actuelle",
            loading_session_history: "Chargement de l’historique",
            messages_suffix: "messages",
            no_matches: "Aucun résultat",
            no_matches_in_window: "Aucun résultat dans la fenêtre actuelle",
            confirmation_required: "Confirmation requise",
            decision_required: "Décision requise",
            question_cancelled: "Sélection annulée",
            question_input_required: "Saisie utilisateur requise",
            answers_received_suffix: "réponses reçues",
            validation_pipeline_rejected: "Les commandes de validation doivent se terminer sans tronquer la sortie avec head ou tail",
            custom_permission_return: "Retour à la saisie principale ; continuez à écrire",
        },
        Language::Ja => InteractionText {
            selection_hint: "↑/↓ で選択 · Enter で確定 · Esc でキャンセル",
            current_label: "現在：",
            question_input_prefix: "  入力",
            question_or_direct_answer: "、または回答を直接入力",
            question_invalid_range_prefix: "  有効な番号を入力してください：1-",
            question_invalid: "  有効な番号、または q でキャンセル",
            custom_answer: "カスタム回答…",
            resume_title: "セッションを再開",
            resume_inline_unsupported: "再開モードではインライン入力を使用できません",
            resume_json_unsupported: "再開モードでは --json を使用できません",
            resume_terminal_only: "再開モードには対話型ターミナルが必要です",
            session_list: "セッション一覧",
            recent_sessions_hint: "最近のセッション · ↑↓ 選択 · Enter 再開 · Esc 終了",
            resume_input_prompt: "番号を入力して Enter（q で終了）：",
            resume_invalid_prefix: "無効です。1-",
            resume_invalid_suffix: "、または q を入力してください",
            empty_value: "<空>",
            no_execution_plan: "このセッションにはまだ実行計画がありません",
            execution_plan: "実行計画",
            completed: "完了",
            complete: "完了",
            in_progress: "実行中",
            pending: "保留",
            current_project_session: "現在のプロジェクトセッション",
            loading_session_history: "セッション履歴を読み込み中",
            messages_suffix: "件のメッセージ",
            no_matches: "一致なし",
            no_matches_in_window: "現在の範囲に一致なし",
            confirmation_required: "確認が必要です",
            decision_required: "選択が必要です",
            question_cancelled: "選択をキャンセルしました",
            question_input_required: "ユーザー入力が必要です",
            answers_received_suffix: "件の回答を受信",
            validation_pipeline_rejected: "検証コマンドは head または tail で出力を切らずに完了させてください",
            custom_permission_return: "メイン入力欄に戻りました。入力を続けてください",
        },
        Language::Ru => InteractionText {
            selection_hint: "↑/↓ выбрать · Enter подтвердить · Esc отменить",
            current_label: "Текущий:",
            question_input_prefix: "  Введите",
            question_or_direct_answer: " или введите ответ",
            question_invalid_range_prefix: "  Введите допустимый номер: 1-",
            question_invalid: "  Введите допустимый номер или q для отмены",
            custom_answer: "Свой ответ…",
            resume_title: "Продолжить сеанс",
            resume_inline_unsupported: "Режим продолжения не поддерживает встроенный запрос",
            resume_json_unsupported: "Режим продолжения не поддерживает --json",
            resume_terminal_only: "Для продолжения нужен интерактивный терминал",
            session_list: "Список сеансов",
            recent_sessions_hint: "Недавние сеансы · ↑↓ выбрать · Enter продолжить · Esc выйти",
            resume_input_prompt: "Введите номер и нажмите Enter (q для выхода):",
            resume_invalid_prefix: "Недопустимый ввод. Введите 1-",
            resume_invalid_suffix: " или q для выхода",
            empty_value: "<пусто>",
            no_execution_plan: "У этого сеанса пока нет плана выполнения",
            execution_plan: "План выполнения",
            completed: "завершено",
            complete: "Готово",
            in_progress: "Выполняется",
            pending: "Ожидает",
            current_project_session: "Текущий сеанс проекта",
            loading_session_history: "Загрузка истории сеанса",
            messages_suffix: "сообщений",
            no_matches: "Совпадений нет",
            no_matches_in_window: "В текущем окне совпадений нет",
            confirmation_required: "Требуется подтверждение",
            decision_required: "Требуется выбор",
            question_cancelled: "Выбор отменён",
            question_input_required: "Требуется ввод пользователя",
            answers_received_suffix: "ответов получено",
            validation_pipeline_rejected: "Команды проверки должны завершаться без обрезки вывода через head или tail",
            custom_permission_return: "Вы вернулись к основному вводу; продолжайте ввод",
        },
        Language::En => InteractionText {
            selection_hint: "Use ↑/↓ · Enter to confirm · Esc to cancel",
            current_label: "Current:",
            question_input_prefix: "  Enter",
            question_or_direct_answer: ", or type an answer",
            question_invalid_range_prefix: "  Enter a valid number from 1-",
            question_invalid: "  Enter a valid number, or q to cancel",
            custom_answer: "Custom answer…",
            resume_title: "Resume session",
            resume_inline_unsupported: "Resume mode does not accept an inline prompt",
            resume_json_unsupported: "Resume mode does not support --json",
            resume_terminal_only: "Resume mode requires an interactive terminal",
            session_list: "Session list",
            recent_sessions_hint: "Recent sessions · ↑↓ select · Enter resume · Esc exit",
            resume_input_prompt: "Enter a number and press Enter (q to exit):",
            resume_invalid_prefix: "Invalid input. Enter 1-",
            resume_invalid_suffix: ", or q to exit",
            empty_value: "<empty>",
            no_execution_plan: "This session does not have an execution plan yet",
            execution_plan: "Execution plan",
            completed: "completed",
            complete: "Complete",
            in_progress: "In progress",
            pending: "Pending",
            current_project_session: "Current project session",
            loading_session_history: "Loading session history",
            messages_suffix: "messages",
            no_matches: "No matches",
            no_matches_in_window: "No matches in the current window",
            confirmation_required: "User confirmation required",
            decision_required: "Your decision is needed",
            question_cancelled: "Selection cancelled",
            question_input_required: "User input required",
            answers_received_suffix: "answers received",
            validation_pipeline_rejected: "Validation commands must finish without truncating output through head or tail",
            custom_permission_return: "Returned to the main composer; continue typing",
        },
    }
}

#[derive(Debug, Clone, Copy)]
enum ToolDisplayError<'a> {
    UnknownTool(&'a str),
    NotFile(&'a str),
    ReadFailed(&'a str),
    ContentRequired,
    ContentExceeds(&'a str),
    CreateParentFailed(&'a str),
    WriteFailed(&'a str),
    OldTextRequired,
    NewTextRequired,
    OldTextNotFound,
    OldTextMatched(&'a str),
    NotDirectory(&'a str),
    ListFailed(&'a str),
    InvalidRegex(&'a str),
    CommandTooLong,
    ShellMetacharacters,
    NoBashPrefixes,
    NotInBashAllowList(&'a str),
    CwdNotDirectory(&'a str),
    TempStdoutFailed(&'a str),
    TempStderrFailed(&'a str),
    ExecFailed(&'a str),
    CommandTimedOut(&'a str),
    WaitFailed(&'a str),
    Required(&'a str),
    NoAllowedDirs,
    FailedResolve(&'a str),
    PathDoesNotExist(&'a str),
    FailedResolveParent(&'a str),
    OutsideAllowed(&'a str),
    InvalidGlob(&'a str),
}

pub fn current_language() -> Language {
    if let Some(language) = LANGUAGE_OVERRIDE.get() {
        return *language;
    }

    for name in ["NOLA_CLI_LANG", "LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = env::var(name) {
            let language = language_from_tag(&value);
            if language != Language::En || is_explicit_english(&value) {
                return language;
            }
        }
    }
    Language::En
}

pub fn language_from_tag(tag: &str) -> Language {
    parse_language_tag(tag).unwrap_or(Language::En)
}

pub fn parse_language_tag(tag: &str) -> Option<Language> {
    let normalized = normalized_locale_tag(tag);
    if normalized == "c" || normalized == "posix" || normalized.is_empty() {
        return None;
    }

    if normalized == "en" || normalized.starts_with("en-") {
        Some(Language::En)
    } else if normalized == "zh" || normalized.starts_with("zh-") {
        Some(Language::ZhHans)
    } else if normalized == "de" || normalized.starts_with("de-") {
        Some(Language::De)
    } else if normalized == "es" || normalized.starts_with("es-") {
        Some(Language::Es)
    } else if normalized == "fr" || normalized.starts_with("fr-") {
        Some(Language::Fr)
    } else if normalized == "ja" || normalized.starts_with("ja-") {
        Some(Language::Ja)
    } else if normalized == "ru" || normalized.starts_with("ru-") {
        Some(Language::Ru)
    } else {
        None
    }
}

pub fn set_language(language: Language) {
    let _ = LANGUAGE_OVERRIDE.set(language);
}

pub fn help_text() -> &'static str {
    help_text_for(current_language())
}

pub fn browser_locale() -> &'static str {
    browser_locale_for(current_language())
}

pub fn browser_accept_language() -> &'static str {
    browser_accept_language_for(current_language())
}

pub fn browser_locale_for(language: Language) -> &'static str {
    match language {
        Language::ZhHans => "zh-CN",
        Language::En => "en-US",
        Language::De => "de-DE",
        Language::Es => "es-ES",
        Language::Fr => "fr-FR",
        Language::Ja => "ja-JP",
        Language::Ru => "ru-RU",
    }
}

pub fn browser_accept_language_for(language: Language) -> &'static str {
    match language {
        Language::ZhHans => "zh-CN,zh;q=0.9,en-US;q=0.8,en;q=0.7",
        Language::En => "en-US,en;q=0.9",
        Language::De => "de-DE,de;q=0.9,en-US;q=0.8,en;q=0.7",
        Language::Es => "es-ES,es;q=0.9,en-US;q=0.8,en;q=0.7",
        Language::Fr => "fr-FR,fr;q=0.9,en-US;q=0.8,en;q=0.7",
        Language::Ja => "ja-JP,ja;q=0.9,en-US;q=0.8,en;q=0.7",
        Language::Ru => "ru-RU,ru;q=0.9,en-US;q=0.8,en;q=0.7",
    }
}

pub fn help_text_for(language: Language) -> &'static str {
    match language {
        Language::ZhHans => {
            r#"Nola CLI

用法：
  nola [消息...] [选项]
  nola init [选项]
  nola wiki [选项]
  nola web [选项]
  nola server [选项]
  nola sessions
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

选项：
  --endpoint URL          OpenAI 兼容端点，通常以 /v1 结尾
  --api-key KEY           API Key；为空时不发送
  --model MODEL           发送给端点的模型名
  --project PATH          项目根目录，默认当前目录
  --allow-dir PATH        允许本地工具访问的目录；可重复
  --bash-prefix PREFIX    允许执行的 Bash 命令前缀；可重复
  --json                  输出按行分隔的 JSON 事件
  --debug                 写入本地调试日志，并开启更详细终端交互（完整状态、工具调用与关键工具参数）
  --lang LANGUAGE         覆盖界面语言 (zh | en | de | es | fr | ja | ru)
  --yes, -y               仅确认 init/wiki 的模型联网请求；不授予 Agent 工具权限
  --network-policy MODE    联网策略：allow、ask、deny（默认 allow）
  --force-tool-set MODE    限制暴露工具：auto、all 或 write（默认 auto）
  --no-tools              禁用本地工具
  --max-iterations N      限制可调用工具的模型请求轮次；0 表示不限
  --session ID            加载/保存指定会话
  --continue, -c          打开恢复会话页并继续最近保存的会话
  --resume                打开恢复会话页（按↑/↓选择）并选择要继续的会话
  --no-save               本次运行不保存到会话
  --output PATH           init 输出 NOLA.md 路径；wiki 输出 wiki 路径
  --force                 覆盖已存在的 init/wiki 输出文件
  --dry-run               预览 init/wiki 输出但不写文件
  --host HOST             web 服务监听地址，默认 127.0.0.1
  --port PORT             web 服务端口，默认 5001
  --web-root PATH         web 资源目录，默认 <project>/web
  --help, -h              显示帮助

环境变量：
  NOLA_CLI_LANG
  NOLA_CLI_PROJECT_ROOT
  NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD
  NOLA_AGENT_CONTEXT_WINDOW_TOKENS
  NOLA_CLI_ALLOWED_DIRS, NOLA_CLI_BASH_PREFIXES
  NOLA_CLI_DEBUG
  NOLA_CLI_DEBUG_LOG_PATH
  NOLA_CLI_DEBUG_LOG_MAX_BYTES, NOLA_CLI_DEBUG_LOG_BACKUPS
  NOLA_CLI_INTERACTION_MODE (auto, always, never)
  NOLA_CLI_NETWORK_POLICY
  NOLA_DB_PATH, NOLA_CLI_PYTHON

配置文件：
  ~/.nola/config.json(.jsonc)
  <project>/.nola/config.json(.jsonc), <project>/nola.json(.jsonc)
  兼容 <project>/.nola/current_config.json(.jsonc)
  支持 llm、allowed_dirs、bash_prefixes、permissions、agents、compaction、external_execution、instructions、debug、language
"#
        }
        Language::En => {
            r#"Nola CLI

Usage:
  nola [message...] [options]
  nola init [options]
  nola wiki [options]
  nola web [options]
  nola server [options]
  nola sessions
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

Options:
  --endpoint URL          OpenAI-compatible endpoint, usually ending in /v1
  --api-key KEY           API key; omitted when empty
  --model MODEL           Model name sent to the endpoint
  --project PATH          Project root, default current directory
  --allow-dir PATH        Local directory tools may access; repeatable
  --bash-prefix PREFIX    Bash command prefix to allow; repeatable
  --json                  Emit newline-delimited JSON events
  --debug                 Write local debug logs and enable detailed terminal interaction (full status + tool details)
  --lang LANGUAGE         Override interface language (zh | en | de | es | fr | ja | ru)
  --yes, -y               Approve init/wiki model-network requests only; never grants Agent tool permission
  --network-policy MODE   Network policy: allow|ask|deny (default allow)
  --force-tool-set MODE   Restrict exposed tools: auto|all|write (default auto)
  --no-tools              Disable local tools
  --max-iterations N      Cap tool-capable model requests; 0 means unlimited
  --session ID            Load/save a named session
  --continue, -c          Open the resume session page and continue the latest saved session
  --resume                Open the resume session page (newest first) and choose one session to continue
  --no-save               Do not save this run to a session
  --output PATH           init NOLA.md output path; wiki output path
  --force                 Overwrite existing init/wiki output files
  --dry-run               Preview init/wiki output without writing files
  --host HOST             Web server bind host, default 127.0.0.1
  --port PORT             Web server port, default 5001
  --web-root PATH         Web asset root, default <project>/web
  --help, -h              Show help

Environment:
  NOLA_CLI_LANG
  NOLA_CLI_NETWORK_POLICY
  NOLA_CLI_PROJECT_ROOT
  NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD
  NOLA_AGENT_CONTEXT_WINDOW_TOKENS
  NOLA_CLI_ALLOWED_DIRS, NOLA_CLI_BASH_PREFIXES
  NOLA_CLI_DEBUG
  NOLA_CLI_DEBUG_LOG_PATH
  NOLA_CLI_DEBUG_LOG_MAX_BYTES, NOLA_CLI_DEBUG_LOG_BACKUPS
  NOLA_CLI_INTERACTION_MODE (auto, always, never)
  NOLA_DB_PATH, NOLA_CLI_PYTHON

Config files:
  ~/.nola/config.json(.jsonc)
  <project>/.nola/config.json(.jsonc), <project>/nola.json(.jsonc)
  Compatible with <project>/.nola/current_config.json(.jsonc)
  Supports llm, allowed_dirs, bash_prefixes, permissions, agents, compaction, external_execution, instructions, debug, language
"#
        }
        Language::De => {
            r#"Nola CLI

Verwendung:
  nola [Nachricht...] [Optionen]
  nola init [Optionen]
  nola wiki [Optionen]
  nola web [Optionen]
  nola server [Optionen]
  nola sessions
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

Optionen:
  --endpoint URL          OpenAI-kompatibler Endpunkt, meist mit /v1 am Ende
  --api-key KEY           API-Schlüssel; leer bedeutet nicht senden
  --model MODEL           Modellname für den Endpunkt
  --project PATH          Projektstamm, standardmäßig das aktuelle Verzeichnis
  --allow-dir PATH        Verzeichnis, auf das lokale Tools zugreifen dürfen; wiederholbar
  --bash-prefix PREFIX    Erlaubter Bash-Befehlspräfix; wiederholbar
  --json                  Zeilengetrennte JSON-Ereignisse ausgeben
  --debug                 Lokale Debug-Protokolle und ausführliche Terminaldetails aktivieren
  --lang LANGUAGE         Sprache der Oberfläche überschreiben (zh | en | de | es | fr | ja | ru)
  --yes, -y               Nur Modell-Netzwerkanfragen für init/wiki bestätigen; keine Agent-Tool-Freigabe
  --network-policy MODE   Netzwerkregel: allow|ask|deny (Standard: allow)
  --force-tool-set MODE   Sichtbare Tools begrenzen: auto|all|write (Standard: auto)
  --no-tools              Lokale Tools deaktivieren
  --max-iterations N      Modellanfragen mit Tools begrenzen; 0 bedeutet unbegrenzt
  --session ID            Benannte Sitzung laden/speichern
  --continue, -c          Sitzungswiederherstellungsseite öffnen und zuletzt gespeicherte Sitzung fortsetzen
  --resume                Sitzungswiederherstellungsseite öffnen (neueste zuerst) und eine Sitzung zum Fortsetzen wählen
  --no-save               Diesen Lauf nicht als Sitzung speichern
  --output PATH           Ausgabepfad für init NOLA.md beziehungsweise wiki
  --force                 Vorhandene init/wiki-Ausgabedateien überschreiben
  --dry-run               init/wiki-Ausgabe nur anzeigen, ohne Dateien zu schreiben
  --host HOST             Bind-Adresse des Webservers, Standard 127.0.0.1
  --port PORT             Port des Webservers, Standard 5001
  --web-root PATH         Stammverzeichnis der Webressourcen, Standard <project>/web
  --help, -h              Hilfe anzeigen

Umgebung:
  NOLA_CLI_LANG
  NOLA_CLI_NETWORK_POLICY
  NOLA_CLI_PROJECT_ROOT
  NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD
  NOLA_AGENT_CONTEXT_WINDOW_TOKENS
  NOLA_CLI_ALLOWED_DIRS, NOLA_CLI_BASH_PREFIXES
  NOLA_CLI_DEBUG, NOLA_CLI_DEBUG_LOG_PATH
  NOLA_CLI_DEBUG_LOG_MAX_BYTES, NOLA_CLI_DEBUG_LOG_BACKUPS
  NOLA_CLI_INTERACTION_MODE (auto, always, never)
  NOLA_DB_PATH, NOLA_CLI_PYTHON

Konfigurationsdateien:
  ~/.nola/config.json(.jsonc)
  <project>/.nola/config.json(.jsonc), <project>/nola.json(.jsonc)
  Kompatibel mit <project>/.nola/current_config.json(.jsonc)
  Unterstützt llm, allowed_dirs, bash_prefixes, permissions, agents, compaction, external_execution, instructions, debug, language
"#
        }
        Language::Es => {
            r#"Nola CLI

Uso:
  nola [mensaje...] [opciones]
  nola init [opciones]
  nola wiki [opciones]
  nola web [opciones]
  nola server [opciones]
  nola sessions
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

Opciones:
  --endpoint URL          Endpoint compatible con OpenAI, normalmente terminado en /v1
  --api-key KEY           Clave API; si está vacía no se envía
  --model MODEL           Nombre del modelo enviado al endpoint
  --project PATH          Raíz del proyecto, por defecto el directorio actual
  --allow-dir PATH        Directorio accesible para herramientas locales; repetible
  --bash-prefix PREFIX    Prefijo de comando Bash permitido; repetible
  --json                  Emitir eventos JSON separados por líneas
  --debug                 Escribir registros locales y mostrar detalles completos en el terminal
  --lang LANGUAGE         Cambiar el idioma de la interfaz (zh | en | de | es | fr | ja | ru)
  --yes, -y               Aprobar solo la red del modelo de init/wiki; no concede permisos de herramientas
  --network-policy MODE   Política de red: allow|ask|deny (predeterminado: allow)
  --force-tool-set MODE   Limitar herramientas visibles: auto|all|write (predeterminado: auto)
  --no-tools              Desactivar herramientas locales
  --max-iterations N      Limitar solicitudes del modelo con herramientas; 0 sin límite
  --session ID            Cargar/guardar una sesión con nombre
  --continue, -c          Abrir la página de recuperación de sesión y continuar la última sesión guardada
  --resume                Abrir la página de recuperación de sesión (de la más reciente a la más antigua) y elegir una para continuar
  --no-save               No guardar esta ejecución en una sesión
  --output PATH           Ruta de salida de init NOLA.md o de wiki
  --force                 Sobrescribir los archivos de salida de init/wiki
  --dry-run               Previsualizar init/wiki sin escribir archivos
  --host HOST             Dirección del servidor web, predeterminada 127.0.0.1
  --port PORT             Puerto del servidor web, predeterminado 5001
  --web-root PATH         Raíz de recursos web, predeterminada <project>/web
  --help, -h              Mostrar ayuda

Entorno:
  NOLA_CLI_LANG
  NOLA_CLI_NETWORK_POLICY
  NOLA_CLI_PROJECT_ROOT
  NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD
  NOLA_AGENT_CONTEXT_WINDOW_TOKENS
  NOLA_CLI_ALLOWED_DIRS, NOLA_CLI_BASH_PREFIXES
  NOLA_CLI_DEBUG, NOLA_CLI_DEBUG_LOG_PATH
  NOLA_CLI_DEBUG_LOG_MAX_BYTES, NOLA_CLI_DEBUG_LOG_BACKUPS
  NOLA_CLI_INTERACTION_MODE (auto, always, never)
  NOLA_DB_PATH, NOLA_CLI_PYTHON

Archivos de configuración:
  ~/.nola/config.json(.jsonc)
  <project>/.nola/config.json(.jsonc), <project>/nola.json(.jsonc)
  Compatible con <project>/.nola/current_config.json(.jsonc)
  Admite llm, allowed_dirs, bash_prefixes, permissions, agents, compaction, external_execution, instructions, debug, language
"#
        }
        Language::Fr => {
            r#"Nola CLI

Utilisation :
  nola [message...] [options]
  nola init [options]
  nola wiki [options]
  nola web [options]
  nola server [options]
  nola sessions
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

Options :
  --endpoint URL          Point de terminaison compatible OpenAI, souvent terminé par /v1
  --api-key KEY           Clé API ; vide signifie ne pas l'envoyer
  --model MODEL           Nom du modèle envoyé au point de terminaison
  --project PATH          Racine du projet, répertoire actuel par défaut
  --allow-dir PATH        Répertoire accessible aux outils locaux ; répétable
  --bash-prefix PREFIX    Préfixe de commande Bash autorisé ; répétable
  --json                  Émettre des événements JSON séparés par lignes
  --debug                 Écrire les journaux locaux et afficher les détails complets du terminal
  --lang LANGUAGE         Remplacer la langue de l'interface (zh | en | de | es | fr | ja | ru)
  --yes, -y               Confirmer le réseau du modèle pour init/wiki uniquement ; aucun droit sur les outils
  --network-policy MODE   Politique réseau : allow|ask|deny (allow par défaut)
  --force-tool-set MODE   Limiter les outils visibles : auto|all|write (auto par défaut)
  --no-tools              Désactiver les outils locaux
  --max-iterations N      Limiter les requêtes du modèle avec outils ; 0 sans limite
  --session ID            Charger/enregistrer une session nommée
  --continue, -c          Ouvrir la page de reprise de session et continuer la dernière session enregistrée
  --resume                Ouvrir la page de reprise de session (plus récentes d'abord) et en choisir une
  --no-save               Ne pas enregistrer cette exécution dans une session
  --output PATH           Chemin de sortie de init NOLA.md ou du wiki
  --force                 Remplacer les fichiers de sortie init/wiki existants
  --dry-run               Prévisualiser init/wiki sans écrire de fichier
  --host HOST             Adresse d'écoute du serveur web, 127.0.0.1 par défaut
  --port PORT             Port du serveur web, 5001 par défaut
  --web-root PATH         Racine des ressources web, <project>/web par défaut
  --help, -h              Afficher l'aide

Environnement :
  NOLA_CLI_LANG
  NOLA_CLI_NETWORK_POLICY
  NOLA_CLI_PROJECT_ROOT
  NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD
  NOLA_AGENT_CONTEXT_WINDOW_TOKENS
  NOLA_CLI_ALLOWED_DIRS, NOLA_CLI_BASH_PREFIXES
  NOLA_CLI_DEBUG, NOLA_CLI_DEBUG_LOG_PATH
  NOLA_CLI_DEBUG_LOG_MAX_BYTES, NOLA_CLI_DEBUG_LOG_BACKUPS
  NOLA_CLI_INTERACTION_MODE (auto, always, never)
  NOLA_DB_PATH, NOLA_CLI_PYTHON

Fichiers de configuration :
  ~/.nola/config.json(.jsonc)
  <project>/.nola/config.json(.jsonc), <project>/nola.json(.jsonc)
  Compatible avec <project>/.nola/current_config.json(.jsonc)
  Prend en charge llm, allowed_dirs, bash_prefixes, permissions, agents, compaction, external_execution, instructions, debug, language
"#
        }
        Language::Ja => {
            r#"Nola CLI

使い方:
  nola [メッセージ...] [オプション]
  nola init [オプション]
  nola wiki [オプション]
  nola web [オプション]
  nola server [オプション]
  nola sessions
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

オプション:
  --endpoint URL          OpenAI 互換エンドポイント。通常は /v1 で終わります
  --api-key KEY           API キー。空の場合は送信しません
  --model MODEL           エンドポイントへ送るモデル名
  --project PATH          プロジェクトルート。既定は現在のディレクトリ
  --allow-dir PATH        ローカルツールがアクセスできるディレクトリ。複数指定可
  --bash-prefix PREFIX    許可する Bash コマンド接頭辞。複数指定可
  --json                  改行区切りの JSON イベントを出力
  --debug                 ローカルデバッグログと詳細なターミナル表示を有効化
  --lang LANGUAGE         インターフェース言語の上書き (zh | en | de | es | fr | ja | ru)
  --yes, -y               init/wiki のモデル通信のみ承認。Agent ツールの権限は付与しません
  --network-policy MODE   ネットワーク方針: allow|ask|deny (既定: allow)
  --force-tool-set MODE   公開ツールを制限: auto|all|write (既定: auto)
  --no-tools              ローカルツールを無効化
  --max-iterations N      ツール利用可能なモデル要求を制限（0 は無制限）
  --session ID            名前付きセッションを読み込み/保存
  --continue, -c          セッション再開ページを開いて、最後に保存したセッションを続行
  --resume                セッション再開ページを開いて（新しい順）続けるものを選択
  --no-save               この実行をセッションに保存しない
  --output PATH           init の NOLA.md または wiki の出力先
  --force                 既存の init/wiki 出力ファイルを上書き
  --dry-run               ファイルを書かずに init/wiki 出力をプレビュー
  --host HOST             Web サーバーの待受アドレス。既定 127.0.0.1
  --port PORT             Web サーバーのポート。既定 5001
  --web-root PATH         Web リソースのルート。既定 <project>/web
  --help, -h              ヘルプを表示

環境変数:
  NOLA_CLI_LANG
  NOLA_CLI_NETWORK_POLICY
  NOLA_CLI_PROJECT_ROOT
  NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD
  NOLA_AGENT_CONTEXT_WINDOW_TOKENS
  NOLA_CLI_ALLOWED_DIRS, NOLA_CLI_BASH_PREFIXES
  NOLA_CLI_DEBUG, NOLA_CLI_DEBUG_LOG_PATH
  NOLA_CLI_DEBUG_LOG_MAX_BYTES, NOLA_CLI_DEBUG_LOG_BACKUPS
  NOLA_CLI_INTERACTION_MODE (auto, always, never)
  NOLA_DB_PATH, NOLA_CLI_PYTHON

設定ファイル:
  ~/.nola/config.json(.jsonc)
  <project>/.nola/config.json(.jsonc), <project>/nola.json(.jsonc)
  <project>/.nola/current_config.json(.jsonc) と互換
  llm, allowed_dirs, bash_prefixes, permissions, agents, compaction, external_execution, instructions, debug, language に対応
"#
        }
        Language::Ru => {
            r#"Nola CLI

Использование:
  nola [сообщение...] [параметры]
  nola init [параметры]
  nola wiki [параметры]
  nola web [параметры]
  nola server [параметры]
  nola sessions
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

Параметры:
  --endpoint URL          OpenAI-совместимый endpoint, обычно оканчивается на /v1
  --api-key KEY           API-ключ; если пустой, не отправляется
  --model MODEL           Имя модели, отправляемое в endpoint
  --project PATH          Корень проекта, по умолчанию текущий каталог
  --allow-dir PATH        Каталог, доступный локальным инструментам; можно повторять
  --bash-prefix PREFIX    Разрешенный префикс Bash-команды; можно повторять
  --json                  Выводить события JSON по одному на строку
  --debug                 Включить локальные журналы и подробный вывод терминала
  --lang LANGUAGE         Перекрывать язык интерфейса (zh | en | de | es | fr | ja | ru)
  --yes, -y               Подтверждать только сеть модели init/wiki; не даёт прав инструментам агента
  --network-policy MODE   Сетевая политика: allow|ask|deny (по умолчанию allow)
  --force-tool-set MODE   Ограничить доступные инструменты: auto|all|write (по умолчанию auto)
  --no-tools              Отключить локальные инструменты
  --max-iterations N      Ограничить запросы модели с инструментами; 0 — без ограничения
  --session ID            Загрузить/сохранить именованный сеанс
  --continue, -c          Открыть страницу восстановления сессии и продолжить последний сохраненный сеанс
  --resume                Открыть страницу восстановления сессии (сначала новые) и выбрать для продолжения
  --no-save               Не сохранять этот запуск в сеанс
  --output PATH           Путь вывода init NOLA.md или wiki
  --force                 Перезаписать существующие файлы init/wiki
  --dry-run               Предпросмотр init/wiki без записи файлов
  --host HOST             Адрес веб-сервера, по умолчанию 127.0.0.1
  --port PORT             Порт веб-сервера, по умолчанию 5001
  --web-root PATH         Корень веб-ресурсов, по умолчанию <project>/web
  --help, -h              Показать справку

Окружение:
  NOLA_CLI_LANG
  NOLA_CLI_NETWORK_POLICY
  NOLA_CLI_PROJECT_ROOT
  NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD
  NOLA_AGENT_CONTEXT_WINDOW_TOKENS
  NOLA_CLI_ALLOWED_DIRS, NOLA_CLI_BASH_PREFIXES
  NOLA_CLI_DEBUG, NOLA_CLI_DEBUG_LOG_PATH
  NOLA_CLI_DEBUG_LOG_MAX_BYTES, NOLA_CLI_DEBUG_LOG_BACKUPS
  NOLA_CLI_INTERACTION_MODE (auto, always, never)
  NOLA_DB_PATH, NOLA_CLI_PYTHON

Файлы конфигурации:
  ~/.nola/config.json(.jsonc)
  <project>/.nola/config.json(.jsonc), <project>/nola.json(.jsonc)
  Совместимо с <project>/.nola/current_config.json(.jsonc)
  Поддерживает llm, allowed_dirs, bash_prefixes, permissions, agents, compaction, external_execution, instructions, debug, language
"#
        }
    }
}

pub fn tr(message: Message) -> &'static str {
    text_for(current_language(), message)
}

pub fn format_error(message: &str) -> String {
    let language = current_language();
    let prefix = text_for(language, Message::ErrorPrefix);
    if matches!(language, Language::ZhHans | Language::Ja) {
        format!("{prefix}{message}")
    } else {
        format!("{prefix} {message}")
    }
}

pub fn localize_tool_error_display(text: &str) -> String {
    localize_tool_error_display_for(current_language(), text)
}

pub fn localize_tool_display_name(name: &str) -> String {
    localize_tool_display_name_for(current_language(), name)
}

pub fn localize_agent_phase(phase: &str) -> String {
    let phase = phase.trim().to_ascii_lowercase().replace('-', "_");
    let language = current_language();
    let review_label = match (language, phase.as_str()) {
        (Language::ZhHans, "answer_review_pending") => Some("待核验正文"),
        (Language::ZhHans, "answer_review_approved") => Some("正文已核验"),
        (Language::ZhHans, "answer_review_rejected") => Some("此版本未通过核验，已失效"),
        (Language::ZhHans, "answer_review_interrupted") => Some("此版本核验已中断，请勿视为最终结论"),
        (_, "answer_review_pending") => Some("Draft — pending review"),
        (_, "answer_review_approved") => Some("Answer verified"),
        (_, "answer_review_rejected") => Some("Draft rejected — superseded"),
        (_, "answer_review_interrupted") => Some("Review interrupted — draft is not final"),
        _ => None,
    };
    if let Some(label) = review_label { return label.to_string(); }
    if phase == "reviewing" {
        return match language {
            Language::ZhHans => "正在核验回答",
            Language::Ja => "回答を確認中",
            Language::De => "Antwort wird geprüft",
            Language::Es => "Verificando respuesta",
            Language::Fr => "Vérification de la réponse",
            Language::Ru => "Проверка ответа",
            Language::En => "Reviewing answer",
        }.to_string();
    }
    if phase == "compacting" {
        return match language {
            Language::ZhHans => "正在整理上下文",
            Language::Ja => "コンテキストを整理中",
            Language::De => "Kontext wird komprimiert",
            Language::Es => "Compactando contexto",
            Language::Fr => "Compression du contexte",
            Language::Ru => "Сжатие контекста",
            Language::En => "Compacting context",
        }.to_string();
    }
    let value = match language {
        Language::ZhHans => match phase.as_str() {
            "diagnosing" => "正在诊断",
            "modifying" => "正在修改",
            "verifying" => "正在验证",
            "correcting" => "正在纠错",
            "awaiting_assessment" => "正在评估结果",
            _ => "正在处理",
        },
        Language::Ja => match phase.as_str() {
            "diagnosing" => "診断中",
            "modifying" => "変更中",
            "verifying" => "検証中",
            "correcting" => "修正中",
            "awaiting_assessment" => "結果を評価中",
            _ => "処理中",
        },
        Language::De => match phase.as_str() {
            "diagnosing" => "Diagnose läuft",
            "modifying" => "Änderungen werden vorgenommen",
            "verifying" => "Überprüfung läuft",
            "correcting" => "Fehler wird korrigiert",
            "awaiting_assessment" => "Ergebnisse werden bewertet",
            _ => "Wird verarbeitet",
        },
        Language::Es => match phase.as_str() {
            "diagnosing" => "Diagnosticando",
            "modifying" => "Aplicando cambios",
            "verifying" => "Verificando",
            "correcting" => "Corrigiendo",
            "awaiting_assessment" => "Evaluando resultados",
            _ => "Procesando",
        },
        Language::Fr => match phase.as_str() {
            "diagnosing" => "Diagnostic en cours",
            "modifying" => "Modification en cours",
            "verifying" => "Vérification en cours",
            "correcting" => "Correction en cours",
            "awaiting_assessment" => "Évaluation des résultats",
            _ => "Traitement en cours",
        },
        Language::Ru => match phase.as_str() {
            "diagnosing" => "Диагностика",
            "modifying" => "Внесение изменений",
            "verifying" => "Проверка",
            "correcting" => "Исправление",
            "awaiting_assessment" => "Оценка результатов",
            _ => "Обработка",
        },
        Language::En => match phase.as_str() {
            "diagnosing" => "Diagnosing",
            "modifying" => "Applying changes",
            "verifying" => "Verifying",
            "correcting" => "Correcting",
            "awaiting_assessment" => "Assessing results",
            _ => "Working",
        },
    };
    value.to_string()
}

pub(crate) fn localize_tool_display_name_for(language: Language, name: &str) -> String {
    let normalized = name.trim().to_ascii_lowercase().replace('-', "_");
    if matches!(
        normalized.as_str(),
        "mcp_resources_read" | "mcp_resource_templates_read" | "mcp_prompts_get"
    ) {
        let action = match normalized.as_str() {
            "mcp_resources_read" => 0,
            "mcp_resource_templates_read" => 1,
            _ => 2,
        };
        return match language {
            Language::ZhHans => ["读取 MCP 资源", "读取 MCP 模板资源", "获取 MCP 提示词"][action],
            Language::De => [
                "MCP-Ressource lesen",
                "MCP-Vorlagenressource lesen",
                "MCP-Prompt abrufen",
            ][action],
            Language::Es => [
                "Leer recurso MCP",
                "Leer recurso de plantilla MCP",
                "Obtener prompt MCP",
            ][action],
            Language::Fr => [
                "Lire la ressource MCP",
                "Lire la ressource modèle MCP",
                "Obtenir le prompt MCP",
            ][action],
            Language::Ja => [
                "MCP リソースを読み取る",
                "MCP テンプレートリソースを読み取る",
                "MCP プロンプトを取得",
            ][action],
            Language::Ru => [
                "Прочитать ресурс MCP",
                "Прочитать ресурс шаблона MCP",
                "Получить промпт MCP",
            ][action],
            Language::En => [
                "Read MCP resource",
                "Read MCP template resource",
                "Get MCP prompt",
            ][action],
        }
        .to_string();
    }
    if matches!(
        normalized.as_str(),
        "search_calendar_events"
            | "create_calendar_event"
            | "search_reminders"
            | "create_reminder"
            | "update_reminder"
    ) {
        let action = match normalized.as_str() {
            "search_calendar_events" => 0,
            "create_calendar_event" => 1,
            "search_reminders" => 2,
            "create_reminder" => 3,
            _ => 4,
        };
        return match language {
            Language::ZhHans => [
                "搜索日历",
                "创建日历事件",
                "搜索提醒事项",
                "创建提醒事项",
                "更新提醒事项",
            ][action],
            Language::De => [
                "Kalender durchsuchen",
                "Kalenderereignis erstellen",
                "Erinnerungen durchsuchen",
                "Erinnerung erstellen",
                "Erinnerung aktualisieren",
            ][action],
            Language::Es => [
                "Buscar en Calendario",
                "Crear evento de calendario",
                "Buscar recordatorios",
                "Crear recordatorio",
                "Actualizar recordatorio",
            ][action],
            Language::Fr => [
                "Rechercher dans Calendrier",
                "Créer un événement",
                "Rechercher des rappels",
                "Créer un rappel",
                "Mettre à jour le rappel",
            ][action],
            Language::Ja => [
                "カレンダーを検索",
                "カレンダー予定を作成",
                "リマインダーを検索",
                "リマインダーを作成",
                "リマインダーを更新",
            ][action],
            Language::Ru => [
                "Поиск в календаре",
                "Создать событие",
                "Поиск напоминаний",
                "Создать напоминание",
                "Обновить напоминание",
            ][action],
            Language::En => [
                "Search calendar",
                "Create calendar event",
                "Search reminders",
                "Create reminder",
                "Update reminder",
            ][action],
        }
        .to_string();
    }
    let key = match normalized.as_str() {
        "bash"
        | "shell"
        | "run_command"
        | "execute_command"
        | "bash_command_review"
        | "bash_background_status"
        | "bash_background_wait"
        | "bash_background_stop"
        | "terminal_input" => "command",
        "read_file" | "read_text_file" | "read_many_files" | "notebook_read" | "file_outline"
        | "read_symbol" => "read_file",
        "write_file" | "append_file" | "create_file" => "write_file",
        "edit_file"
        | "edit_file_lines"
        | "multi_edit_file"
        | "notebook_edit"
        | "apply_patch"
        | "replace_text"
        | "replace_symbol_body"
        | "replace_in_files"
        | "rename_symbol"
        | "format_files" => "edit_file",
        "move_file" => "edit_file",
        "delete_file" | "remove_file" => "delete_file",
        "list_dir" | "list_directory" | "file_tree" | "find_files" | "find_up" => "list_folder",
        "project_overview" | "repo_map" => "project_overview",
        "language_service_list" | "language_service_query" => "search_files",
        "codebase_search" | "codebase_text_search" | "codebase_trace" | "codebase_read_symbol" => {
            "search_files"
        }
        "codebase_index"
        | "codebase_index_status"
        | "codebase_projects"
        | "codebase_architecture"
        | "codebase_query"
        | "codebase_schema"
        | "codebase_check_coverage"
        | "codebase_changed_impact"
        | "codebase_delete_index"
        | "codebase_adr"
        | "codebase_ingest_traces" => "workspace",
        "grep"
        | "glob"
        | "search_files"
        | "ripgrep"
        | "code_search"
        | "workspace_symbols"
        | "symbol_search"
        | "implementation_search"
        | "code_navigation"
        | "call_graph"
        | "failure_context"
        | "failure_triage"
        | "file_dependencies"
        | "related_files"
        | "document_search" => "search_files",
        "workspace_status"
        | "code_diagnostics"
        | "coding_conventions"
        | "instruction_context"
        | "skill_invoke"
        | "Skill"
        | "skill_skip"
        | "agent_context"
        | "command_context"
        | "agent_config_context"
        | "ci_context"
        | "context_pack"
        | "focused_context"
        | "impact_analysis"
        | "validation_plan"
        | "test_context"
        | "format_plan"
        | "verification_review"
        | "handoff_context"
        | "workspace_checkpoint" => "workspace",
        "git_diff" | "change_review" | "diff_review" | "changed_symbols" | "git_history"
        | "git_blame" | "git_show" => "changes",
        "web_search" => "web_search",
        "web_fetch" | "fetch_url" => "web_read",
        "tool_search" => "search_tools",
        "tool_output_list" | "tool_output_read" | "tool_output_search" | "tool_output_find" => {
            "read_output"
        }
        "memory_save" | "memory_search" => "memory",
        "spawn_agent" | "nola_orchestrate" => "subtask",
        "list_tasks" | "read_task" | "wait_task" | "stop_task" => "background",
        "propose_agent_plan" | "update_agent_todos" | "agent_plan_file" => "plan",
        "system_time" => "time",
        value if value.starts_with("chrome_") || value.starts_with("browser_") => "browser",
        value if value.starts_with("computer_") => "computer",
        value if value.starts_with("skill_") => "skill",
        value if value.starts_with("mcp_") => "mcp",
        value if value.starts_with("background_") => "background",
        _ => "unknown",
    };
    let translated = match language {
        Language::ZhHans => match key {
            "command" => "运行命令",
            "read_file" => "读取文件",
            "write_file" => "写入文件",
            "edit_file" => "编辑文件",
            "delete_file" => "删除文件",
            "list_folder" => "浏览目录",
            "search_files" => "搜索文件",
            "workspace" => "检查工作区",
            "changes" => "检查变更",
            "project_overview" => "查看项目概览",
            "web_search" => "联网搜索",
            "web_read" => "读取网页",
            "read_output" => "读取工具输出",
            "search_tools" => "搜索工具",
            "memory" => "管理记忆",
            "subtask" => "执行子任务",
            "plan" => "更新计划",
            "time" => "读取系统时间",
            "browser" => "控制浏览器",
            "computer" => "操作电脑",
            "skill" => "使用技能",
            "mcp" => "调用 MCP",
            "background" => "后台任务",
            _ => "工具",
        },
        Language::Ja => match key {
            "command" => "コマンドを実行",
            "read_file" => "ファイルを読む",
            "write_file" => "ファイルを書く",
            "edit_file" => "ファイルを編集",
            "delete_file" => "ファイルを削除",
            "list_folder" => "フォルダを表示",
            "search_files" => "ファイルを検索",
            "workspace" => "ワークスペースを確認",
            "changes" => "変更を確認",
            "project_overview" => "プロジェクト概要を確認",
            "web_search" => "Web を検索",
            "web_read" => "Web ページを読む",
            "read_output" => "ツール出力を読む",
            "search_tools" => "ツールを検索",
            "memory" => "メモリを管理",
            "subtask" => "サブタスクを実行",
            "plan" => "計画を更新",
            "time" => "システム時刻を読む",
            "browser" => "ブラウザを操作",
            "computer" => "コンピューターを操作",
            "skill" => "スキルを使用",
            "mcp" => "MCP を呼び出す",
            "background" => "バックグラウンドタスク",
            _ => "ツール",
        },
        Language::De => match key {
            "command" => "Befehl ausführen",
            "read_file" => "Datei lesen",
            "write_file" => "Datei schreiben",
            "edit_file" => "Datei bearbeiten",
            "delete_file" => "Datei löschen",
            "list_folder" => "Ordner anzeigen",
            "search_files" => "Dateien suchen",
            "workspace" => "Arbeitsbereich prüfen",
            "changes" => "Änderungen prüfen",
            "project_overview" => "Projektübersicht prüfen",
            "web_search" => "Web durchsuchen",
            "web_read" => "Webseite lesen",
            "read_output" => "Werkzeugausgabe lesen",
            "search_tools" => "Werkzeuge suchen",
            "memory" => "Speicher verwalten",
            "subtask" => "Teilaufgabe ausführen",
            "plan" => "Plan aktualisieren",
            "time" => "Systemzeit lesen",
            "browser" => "Browser steuern",
            "computer" => "Computer bedienen",
            "skill" => "Skill verwenden",
            "mcp" => "MCP aufrufen",
            "background" => "Hintergrundaufgabe",
            _ => "Werkzeug",
        },
        Language::Es => match key {
            "command" => "Ejecutar comando",
            "read_file" => "Leer archivo",
            "write_file" => "Escribir archivo",
            "edit_file" => "Editar archivo",
            "delete_file" => "Eliminar archivo",
            "list_folder" => "Ver carpeta",
            "search_files" => "Buscar archivos",
            "workspace" => "Revisar espacio de trabajo",
            "changes" => "Revisar cambios",
            "project_overview" => "Revisar resumen del proyecto",
            "web_search" => "Buscar en la web",
            "web_read" => "Leer página web",
            "read_output" => "Leer salida de herramienta",
            "search_tools" => "Buscar herramientas",
            "memory" => "Gestionar memoria",
            "subtask" => "Ejecutar subtarea",
            "plan" => "Actualizar plan",
            "time" => "Leer hora del sistema",
            "browser" => "Controlar navegador",
            "computer" => "Controlar el ordenador",
            "skill" => "Usar skill",
            "mcp" => "Invocar MCP",
            "background" => "Tarea en segundo plano",
            _ => "Herramienta",
        },
        Language::Fr => match key {
            "command" => "Exécuter une commande",
            "read_file" => "Lire le fichier",
            "write_file" => "Écrire le fichier",
            "edit_file" => "Modifier le fichier",
            "delete_file" => "Supprimer le fichier",
            "list_folder" => "Afficher le dossier",
            "search_files" => "Rechercher des fichiers",
            "workspace" => "Vérifier l’espace de travail",
            "changes" => "Vérifier les modifications",
            "project_overview" => "Vérifier l’aperçu du projet",
            "web_search" => "Rechercher sur le Web",
            "web_read" => "Lire la page Web",
            "read_output" => "Lire la sortie de l’outil",
            "search_tools" => "Rechercher des outils",
            "memory" => "Gérer la mémoire",
            "subtask" => "Exécuter une sous-tâche",
            "plan" => "Mettre à jour le plan",
            "time" => "Lire l’heure système",
            "browser" => "Contrôler le navigateur",
            "computer" => "Contrôler l’ordinateur",
            "skill" => "Utiliser le skill",
            "mcp" => "Appeler MCP",
            "background" => "Tâche en arrière-plan",
            _ => "Outil",
        },
        Language::Ru => match key {
            "command" => "Выполнить команду",
            "read_file" => "Прочитать файл",
            "write_file" => "Записать файл",
            "edit_file" => "Изменить файл",
            "delete_file" => "Удалить файл",
            "list_folder" => "Показать папку",
            "search_files" => "Найти файлы",
            "workspace" => "Проверить рабочую область",
            "changes" => "Проверить изменения",
            "project_overview" => "Проверить обзор проекта",
            "web_search" => "Поиск в интернете",
            "web_read" => "Прочитать веб-страницу",
            "read_output" => "Прочитать вывод инструмента",
            "search_tools" => "Найти инструменты",
            "memory" => "Управлять памятью",
            "subtask" => "Выполнить подзадачу",
            "plan" => "Обновить план",
            "time" => "Прочитать системное время",
            "browser" => "Управлять браузером",
            "computer" => "Управлять компьютером",
            "skill" => "Использовать навык",
            "mcp" => "Вызвать MCP",
            "background" => "Фоновая задача",
            _ => "Инструмент",
        },
        Language::En => match key {
            "command" => "Run command",
            "read_file" => "Read file",
            "write_file" => "Write file",
            "edit_file" => "Edit file",
            "delete_file" => "Delete file",
            "list_folder" => "List folder",
            "search_files" => "Search files",
            "workspace" => "Inspect workspace",
            "changes" => "Inspect changes",
            "project_overview" => "Inspect project overview",
            "web_search" => "Search web",
            "web_read" => "Read webpage",
            "read_output" => "Read tool output",
            "search_tools" => "Search tools",
            "memory" => "Manage memory",
            "subtask" => "Run subtask",
            "plan" => "Update plan",
            "time" => "Read system time",
            "browser" => "Control browser",
            "computer" => "Control computer",
            "skill" => "Use skill",
            "mcp" => "Call MCP",
            "background" => "Background task",
            _ => "Tool",
        },
    };
    if key == "unknown" && !name.trim().is_empty() {
        format!("{translated} · {}", name.trim())
    } else {
        translated.to_string()
    }
}

fn localize_tool_error_display_for(language: Language, text: &str) -> String {
    if language == Language::En {
        return text.to_string();
    }
    parse_tool_display_error(text)
        .map(|error| format_tool_display_error(language, error))
        .unwrap_or_else(|| text.to_string())
}

fn text_for(language: Language, message: Message) -> &'static str {
    match language {
        Language::ZhHans => zh_hans(message),
        Language::En => en(message),
        Language::De => de(message),
        Language::Es => es(message),
        Language::Fr => fr(message),
        Language::Ja => ja(message),
        Language::Ru => ru(message),
    }
}

fn en(message: Message) -> &'static str {
    match message {
        Message::ErrorPrefix => "error:",
        Message::NoSavedSessions => "No saved sessions.",
        Message::MessagesUnit => "messages",
        Message::SessionLabel => "session:",
        Message::CreatedLabel => "created:",
        Message::UpdatedLabel => "updated:",
        Message::MessagesLabel => "messages:",
        Message::DeletedSessionPrefix => "Deleted session",
        Message::FailedReadPromptPrefix => "failed to read prompt:",
        Message::PromptRequired => "message is required",
        Message::SessionRequiresAction => "session requires an action: show or delete",
        Message::RequiresSessionIdSuffix => "requires a session id",
        Message::UnknownSessionActionPrefix => "unknown session action:",
        Message::UnknownCommandPrefix => "unknown command:",
        Message::MaxIterationsPositive => "--max-iterations must be a non-negative integer",
        Message::UnknownOptionPrefix => "unknown option:",
        Message::UnexpectedSessionArgs => "unexpected extra arguments for session command",
        Message::MissingEndpoint => "missing --endpoint or llm.endpoint in a Nola config file",
        Message::ContinueOrSession => "use either --continue or --session, not both",
        Message::CurrentDirFailedPrefix => "current_dir failed:",
        Message::RequiresValueSuffix => "requires a value",
        Message::FailedResolvePathPrefix => "failed to resolve",
        Message::ActiveSessionLabel => "session:",
        Message::FailedCreateSessionDirPrefix => "failed to create session dir:",
        Message::FailedReadSessionPrefix => "failed to read session",
        Message::FailedParseSessionPrefix => "failed to parse session",
        Message::FailedSerializeSessionPrefix => "failed to serialize session:",
        Message::FailedSaveSessionPrefix => "failed to save session:",
        Message::FailedReadSessionsPrefix => "failed to read sessions:",
        Message::FailedReadSessionEntryPrefix => "failed to read session entry:",
        Message::FailedDeleteSessionPrefix => "failed to delete session",
        Message::HomeNotSet => "HOME is not set; cannot save sessions",
        Message::FailedReadLatestSessionPrefix => "failed to read latest session:",
        Message::FailedSaveLatestSessionPrefix => "failed to save latest session:",
        Message::InvalidSessionId => "session id may only contain letters, numbers, '-' and '_'",
        Message::FailedCreateHttpClientPrefix => "failed to create HTTP client:",
        Message::LlmRequestFailedPrefix => "LLM request failed",
        Message::LlmStreamReadFailedPrefix => "LLM stream read failed:",
        Message::ToolRequiresApprovalPrefix => "Tool",
        Message::ToolRequiresApprovalSuffix => "requires approval.",
        Message::ArgsLabel => "args:",
        Message::AllowOncePrompt => "Allow once? [y/N] ",
        Message::PermissionDeniedByUser => "Permission denied by user",
        Message::PermissionDeniedGuidance => {
            "Permission denied; the action did not run. Use /allow to review permissions. Change explicit deny rules in the configuration, then explicitly request a retry. System permissions still apply."
        }
        Message::SubAgentLabel => "sub-agent:",
        Message::TaskLabel => "task:",
        Message::CapabilityLabel => "capability:",
        Message::PromptLabel => "prompt:",
        Message::DelegateAgentCapability => {
            "may run commands or modify files within configured permissions"
        }
        Message::DelegateAgentConfirmationRequired => {
            "Sub-agent approval is required. Obtain explicit approval; each tool remains subject to its own resource permission."
        }
        Message::NetworkAskNonInteractive => {
            "Network request blocked (policy=ask) in non-interactive mode. Use --network-policy=allow to continue."
        }
        Message::NetworkAskPrompt => "Allow model network request now? [y/N]: ",
        Message::NetworkAllowedByUser => "Network request allowed for model call by user.",
        Message::NetworkDeniedByUser => "Model network request denied by user.",
        Message::DisplayReadPrefix => "Read",
        Message::DisplayWrotePrefix => "Wrote",
        Message::DisplayEditedPrefix => "Edited",
        Message::DisplayListedPrefix => "Listed",
        Message::EntriesUnit => "entries",
        Message::MatchesUnit => "matches",
        Message::ReachedMaxIterationsPrefix => "\n\nReached max iterations (",
        Message::ReachedMaxIterationsSuffix => "); returning the current result.",
        Message::InteractiveQueuedRequest => "  queued request:",
        Message::InteractiveRunning => "  running...",
        Message::InteractiveReadyForNextRequest => "  ready for next request",
        Message::InteractiveModelLabel => "Model",
        Message::InteractiveParamsLabel => "Params",
        Message::InteractiveWorkingDirLabel => "Working dir",
        Message::InteractiveWorkspaceLabel => "Workspace",
        Message::InteractiveProviderLabel => "Provider",
        Message::InteractiveProviderLocal => "local config",
        Message::InteractiveModeFieldLabel => "Mode",
        Message::InteractiveNetworkFieldLabel => "Network",
        Message::InteractiveEndpointLabel => "Endpoint",
        Message::InteractiveAvailableModels => "Available models:",
        Message::InteractiveModelUsageHint => "Usage: /model <name>",
        Message::InteractiveAvailableCommands => "Available commands:",
        Message::InteractiveSlashHelpHint => "Show help",
        Message::InteractiveSlashModelHint => "List models, /model <name> to switch",
        Message::InteractiveSlashExecutorHint => "List or switch the core execution unit",
        Message::InteractiveAvailableExecutors => "Execution units:",
        Message::InteractiveExecutorUsage => "Usage: /executor <nola|codex|claude_code|opencode>",
        Message::InteractiveExecutorSelected => "Execution unit selected:",
        Message::InteractiveExecutorUnavailable => "Execution unit unavailable:",
        Message::InteractiveExecutorLabel => "Executor",
        Message::InteractiveSlashDebugHint => "Show or toggle debug details",
        Message::InteractiveSlashDebugUsage => "Usage: /debug on | /debug off",
        Message::InteractiveSlashAllowHint => "Set permission mode",
        Message::InteractiveSlashAllowUsage => "Usage: /allow — open permission selection panel",
        Message::InteractiveAllowPanelTitle => "Permission Mode",
        Message::InteractiveAllowPanelCautious => "Cautious",
        Message::InteractiveAllowPanelCautiousDesc => "Confirm deletion, overwrites, sensitive data access, outgoing content, and new connections. Ordinary reads and new files proceed.",
        Message::InteractiveAllowPanelAllowAll => "Allow All",
        Message::InteractiveAllowPanelAllowAllDesc => "No Nola approval prompts in this conversation. System permissions still apply.",
        Message::InteractiveAllowPanelSelected => "Selected:",
        Message::InteractiveAllowPanelCancelled => "Cancelled",
        Message::InteractiveSlashGoalHint => "Autonomously work toward a goal",
        Message::InteractiveSlashGoalUsage => "Usage: /goal <goal> [-c/--constraints <constraints>] | /goal status | /goal resume | /goal off",
        Message::InteractiveSlashInitHint => "Generate NOLA.md project memory (--dry-run, --force, --output PATH)",
        Message::InteractiveSlashCdHint => "Switch workspace and start a new project session (/cd - to return)",
        Message::InteractiveSlashCompactHint => "Compact model context while preserving full replay history",
        Message::InteractiveSlashNewHint => "Start a clean session in the current workspace",
        Message::InteractiveSlashPlanHint => "Show the current session execution plan",
        Message::InteractiveSlashJobsHint => "List, inspect, or stop persisted background tasks",
        Message::InteractiveSlashResumeHint => "Open the session restore page",
        Message::InteractiveSlashContinueHint => "Continue the latest project session",
        Message::InteractiveSlashHistoryHint => "Re-print full session history",
        Message::InteractiveSlashStatusHint => "Show current runtime state",
        Message::InteractiveSlashExitHint => "Exit current session",
        Message::InteractiveSlashQuitHint => "q/quit/exit also exits",
        Message::InteractiveWelcomeTagline => "local-first assistant",
        Message::InteractiveWelcomeHint => "Type /help for commands · use /model to switch models",
        Message::InteractiveAutoValue => "auto",
        Message::InteractiveNetworkAllowValue => "allow",
        Message::InteractiveNetworkAskValue => "ask",
        Message::InteractiveNetworkDenyValue => "deny",
        Message::InteractiveContextLabel => "Context",
        Message::InteractiveTokensLabel => "tokens",
        Message::InteractiveTokensUnit => "tokens",
        Message::InteractiveMessagesUnit => "messages",
        Message::InteractiveAllowPanelErrorPrefix => "permission panel error:",
        Message::InteractiveGoalStatusPrefix => "goal:",
        Message::InteractiveGoalStatePrefix => "state:",
        Message::InteractiveGoalStateRunning => "running",
        Message::InteractiveGoalStatePaused => "paused",
        Message::InteractiveGoalStateNeedsUser => "waiting for user",
        Message::InteractiveGoalStateFailed => "failed",
        Message::InteractiveGoalResumeStarted => "resuming goal:",
        Message::InteractiveGoalNoResumable => "No resumable goal is available in this session.",
        Message::InteractiveWorkspaceStatusPrefix => "workspace:",
        Message::InteractiveCdUsage => "Usage: /cd PATH · /cd - returns to the previous workspace",
        Message::InteractiveCompactTitle => "context compaction",
        Message::InteractiveCompactNoActiveSession => "no active session",
        Message::InteractiveCompactAlready => "already compact",
        Message::InteractiveCompactCompleted => "compacted",
        Message::InteractiveCompactSummarized => "summarized",
        Message::InteractiveCompactKeptRecent => "kept recent",
        Message::InteractiveCompactSaved => "saved",
        Message::InteractiveCompactHistoryPreserved => "full transcript is preserved for replay",
        Message::InteractiveCompactFailedPrefix => "context compaction failed:",
        Message::InteractiveNewSessionTitle => "new session",
        Message::InteractiveNewSessionPending => "a clean session will start with the next message",
        Message::InteractiveJobsTitle => "background tasks",
        Message::InteractiveJobsUsage => "Usage: /jobs · /jobs ID · /jobs status ID · /jobs stop ID",
        Message::InteractiveUnknownCommandHint => "Unknown command; type /help to see available commands",
        Message::InteractiveChoiceLabel => "Please choose",
        Message::InteractiveModelStatusPrefix => "model:",
        Message::InteractiveDebugStatusPrefix => "debug:",
        Message::InteractiveAllowStatusPrefix => "allow:",
        Message::InteractiveDefaultModelLabel => "(default)",
        Message::InteractiveTurnsLabel => "turns",
        Message::InteractiveToolsLabel => "tools",
        Message::InteractiveNetworkLabel => "network",
        Message::InteractiveInputPlaceholder => "Type a message…",
        Message::InteractiveExecutionDraftPrefix => "↳",
        Message::InteractiveExecutionQueuedPrefix => "Queued follow-up instructions:",
        Message::InteractiveExecutionInterruptHint => "Esc interrupts the current run",
        Message::InteractiveExecutionInputHint => {
            "Esc interrupt · Enter queue follow-up"
        }
        Message::AgentStatusThinking => "thinking",
        Message::AgentStatusRetryingModel => "retrying model response",
        Message::AgentStatusStrictToolStrategy => "strict tool strategy",
        Message::AgentStatusChooseFromAvailableTools => "choose from available tools",
        Message::AgentStatusReadyToContinue => "ready to continue",
        Message::AgentStatusEnteredMode => "entered mode",
        Message::AgentStatusReplyingDirectly => "replying directly",
        Message::AgentStatusExecutingPlan => "executing plan",
        Message::AgentStatusWaitingForUserConfirmation => "waiting for user confirmation",
        Message::AgentStatusProcessing => "processing",
        Message::AgentStatusPreparing => "preparing",
        Message::AgentStatusModelUnavailable => "model temporarily unavailable",
        Message::AgentDoneLabel => "done",
        Message::AgentErrorLabel => "error",
        Message::AgentToolLabel => "tool",
        Message::AgentAgentLabel => "agent",
        Message::AgentSessionLabel => "session",
        Message::AgentStatusLabel => "status",
        Message::AgentInfoLabel => "info",
        Message::AgentReadyCurrentModePrefix => "ready, current mode is",
        Message::AgentReadyCurrentModeSuffix => "",
        Message::AgentStatusDetailsSeparator => ", ",
        Message::AgentPreparingToExecutePrefix => "preparing to execute",
        Message::AgentActionCountSuffixSingle => "action",
        Message::AgentActionCountSuffixPlural => "actions",
        Message::AgentTurnPrefix => "turn",
        Message::AgentTurnSuffix => "",
        Message::AgentInspectingProject => "inspecting project",
        Message::AgentEditingFiles => "editing files",
        Message::AgentRunningCommand => "running command",
        Message::AgentAccessingNetwork => "accessing network",
        Message::AgentSearchingTools => "searching tools",
        Message::AgentStartingSubtask => "starting subtask",
        Message::AgentFileModified => "file modified",
        Message::AgentCommandCompleted => "command completed",
        Message::AgentNetworkRequestCompleted => "network request completed",
        Message::AgentSubtaskCompleted => "subtask completed",
        Message::AgentFailedToModifyFile => "failed to modify file",
        Message::AgentCommandFailed => "command failed",
        Message::AgentNetworkRequestFailed => "network request failed",
        Message::AgentSubtaskFailed => "subtask failed",
        Message::AgentOperationFailed => "operation failed",
        Message::InteractiveUnknownPath => "(unknown)",
        Message::InteractiveStatusOn => "on",
        Message::InteractiveStatusOff => "off",
        Message::InteractiveInvalidLanguageValue => "unsupported language",
        Message::InvalidPortValue => "--port must be a number between 1 and 65535",
        Message::UnknownAgentModePrefix => "unknown --agent-mode",
        Message::NetworkPolicyMustBeOneOf => "--network-policy must be one of: allow, ask, deny",
        Message::ForceToolSetMustBeOneOf => "--force-tool-set must be one of: auto, all, write",
        Message::FailedReadKeyEventPrefix => "failed to read key event:",
        Message::FailedFlushPromptPrefix => "failed to flush prompt:",
        Message::InteractiveRawModeFailedPrefix => "failed to enable raw mode:",
    }
}

fn zh_hans(message: Message) -> &'static str {
    match message {
        Message::ErrorPrefix => "错误：",
        Message::NoSavedSessions => "暂无保存的会话。",
        Message::MessagesUnit => "条消息",
        Message::SessionLabel => "会话：",
        Message::CreatedLabel => "创建时间：",
        Message::UpdatedLabel => "更新时间：",
        Message::MessagesLabel => "消息数：",
        Message::DeletedSessionPrefix => "已删除会话",
        Message::FailedReadPromptPrefix => "读取提示词失败：",
        Message::PromptRequired => "请输入消息内容",
        Message::SessionRequiresAction => "session 需要指定操作：show 或 delete",
        Message::RequiresSessionIdSuffix => "需要会话 ID",
        Message::UnknownSessionActionPrefix => "未知 session 操作：",
        Message::UnknownCommandPrefix => "未知命令：",
        Message::MaxIterationsPositive => "--max-iterations 必须是非负整数",
        Message::UnknownOptionPrefix => "未知选项：",
        Message::UnexpectedSessionArgs => "session 命令不接受额外参数",
        Message::MissingEndpoint => "缺少 --endpoint 或 Nola 配置文件中的 llm.endpoint",
        Message::ContinueOrSession => "--continue 和 --session 只能二选一",
        Message::CurrentDirFailedPrefix => "读取当前目录失败：",
        Message::RequiresValueSuffix => "需要提供值",
        Message::FailedResolvePathPrefix => "解析路径失败",
        Message::ActiveSessionLabel => "会话：",
        Message::FailedCreateSessionDirPrefix => "创建会话目录失败：",
        Message::FailedReadSessionPrefix => "读取会话失败",
        Message::FailedParseSessionPrefix => "解析会话失败",
        Message::FailedSerializeSessionPrefix => "序列化会话失败：",
        Message::FailedSaveSessionPrefix => "保存会话失败：",
        Message::FailedReadSessionsPrefix => "读取会话列表失败：",
        Message::FailedReadSessionEntryPrefix => "读取会话条目失败：",
        Message::FailedDeleteSessionPrefix => "删除会话失败",
        Message::HomeNotSet => "未设置 HOME，无法保存会话",
        Message::FailedReadLatestSessionPrefix => "读取最近会话失败：",
        Message::FailedSaveLatestSessionPrefix => "保存最近会话失败：",
        Message::InvalidSessionId => "会话 ID 只能包含字母、数字、'-' 和 '_'",
        Message::FailedCreateHttpClientPrefix => "创建 HTTP 客户端失败：",
        Message::LlmRequestFailedPrefix => "LLM 请求失败",
        Message::LlmStreamReadFailedPrefix => "读取 LLM 流失败：",
        Message::ToolRequiresApprovalPrefix => "工具",
        Message::ToolRequiresApprovalSuffix => "需要确认。",
        Message::ArgsLabel => "参数：",
        Message::AllowOncePrompt => "允许本次执行？[y/N] ",
        Message::PermissionDeniedByUser => "用户已拒绝授权",
        Message::PermissionDeniedGuidance => {
            "权限未允许，操作未执行。输入 /allow 查看权限设置；配置中的显式 deny 规则需先修改对应配置，再明确要求重试。系统权限约束仍然有效。"
        }
        Message::SubAgentLabel => "子代理：",
        Message::TaskLabel => "任务：",
        Message::CapabilityLabel => "能力：",
        Message::PromptLabel => "提示词：",
        Message::DelegateAgentCapability => "可在已配置权限内执行命令或修改文件",
        Message::DelegateAgentConfirmationRequired => {
            "需要明确批准子代理；其每个工具仍须遵守各自的资源权限。"
        }
        Message::NetworkAskNonInteractive => {
            "非交互模式下已阻止网络请求（policy=ask）。请使用 --network-policy=allow 继续。"
        }
        Message::NetworkAskPrompt => "是否允许本次模型网络请求？[y/N]: ",
        Message::NetworkAllowedByUser => "用户已允许本次模型网络请求。",
        Message::NetworkDeniedByUser => "用户已拒绝本次模型网络请求。",
        Message::DisplayReadPrefix => "已读取",
        Message::DisplayWrotePrefix => "已写入",
        Message::DisplayEditedPrefix => "已编辑",
        Message::DisplayListedPrefix => "已列出",
        Message::EntriesUnit => "项",
        Message::MatchesUnit => "个匹配",
        Message::ReachedMaxIterationsPrefix => "\n\n已达到最大轮次（",
        Message::ReachedMaxIterationsSuffix => "），返回当前结果。",
        Message::InteractiveQueuedRequest => "  已排队请求：",
        Message::InteractiveRunning => "  执行中...",
        Message::InteractiveReadyForNextRequest => "  准备好处理下一个请求",
        Message::InteractiveModelLabel => "模型",
        Message::InteractiveParamsLabel => "参数",
        Message::InteractiveWorkingDirLabel => "工作目录",
        Message::InteractiveWorkspaceLabel => "工作区",
        Message::InteractiveProviderLabel => "提供方",
        Message::InteractiveProviderLocal => "本地配置",
        Message::InteractiveModeFieldLabel => "模式",
        Message::InteractiveNetworkFieldLabel => "网络",
        Message::InteractiveEndpointLabel => "端点",
        Message::InteractiveAvailableModels => "可用模型：",
        Message::InteractiveModelUsageHint => "用法：/model <name>",
        Message::InteractiveAvailableCommands => "可用命令：",
        Message::InteractiveSlashHelpHint => "显示帮助",
        Message::InteractiveSlashModelHint => "列出模型，/model <name> 切换模型",
        Message::InteractiveSlashExecutorHint => "列出或切换核心执行单元",
        Message::InteractiveAvailableExecutors => "执行单元：",
        Message::InteractiveExecutorUsage => "用法：/executor <nola|codex|claude_code|opencode>",
        Message::InteractiveExecutorSelected => "已选择执行单元：",
        Message::InteractiveExecutorUnavailable => "执行单元不可用：",
        Message::InteractiveExecutorLabel => "执行单元",
        Message::InteractiveSlashDebugHint => "开启或关闭详细日志",
        Message::InteractiveSlashDebugUsage => "用法：/debug on | /debug off",
        Message::InteractiveSlashAllowHint => "设置权限模式",
        Message::InteractiveSlashAllowUsage => "用法：/allow — 打开权限选择面板",
        Message::InteractiveAllowPanelTitle => "权限模式",
        Message::InteractiveAllowPanelCautious => "谨慎",
        Message::InteractiveAllowPanelCautiousDesc => "删除、覆盖、敏感数据访问、外发内容和新连接先确认；普通读取与新建直接执行。",
        Message::InteractiveAllowPanelAllowAll => "全部允许",
        Message::InteractiveAllowPanelAllowAllDesc => "此会话内无需 Nola 确认，系统权限仍然生效。",
        Message::InteractiveAllowPanelSelected => "已选择：",
        Message::InteractiveAllowPanelCancelled => "已取消",
        Message::InteractiveSlashGoalHint => "让模型自主达成目标",
        Message::InteractiveSlashGoalUsage => {
            "用法：/goal <目标> [-c/--constraints <约束>] | /goal status | /goal resume | /goal off"
        }
        Message::InteractiveSlashInitHint => {
            "生成 NOLA.md 项目记忆（--dry-run、--force、--output PATH）"
        }
        Message::InteractiveSlashCdHint => "切换工作区并新建项目会话（/cd - 返回上一个工作区）",
        Message::InteractiveSlashCompactHint => "压缩模型上下文，同时保留完整回放历史",
        Message::InteractiveSlashNewHint => "在当前工作区开始全新会话",
        Message::InteractiveSlashPlanHint => "显示当前会话的执行计划",
        Message::InteractiveSlashJobsHint => "列出、查看或停止后台任务",
        Message::InteractiveSlashResumeHint => "打开会话恢复页面",
        Message::InteractiveSlashContinueHint => "继续当前项目的最近会话",
        Message::InteractiveSlashHistoryHint => "重新显示完整会话历史",
        Message::InteractiveSlashStatusHint => "显示当前运行状态",
        Message::InteractiveSlashExitHint => "退出当前会话",
        Message::InteractiveSlashQuitHint => "q/quit/exit 也会退出",
        Message::InteractiveWelcomeTagline => "本地优先个人助手",
        Message::InteractiveWelcomeHint => "输入 /help 查看命令 · 使用 /model 切换模型",
        Message::InteractiveAutoValue => "自动",
        Message::InteractiveNetworkAllowValue => "允许",
        Message::InteractiveNetworkAskValue => "询问",
        Message::InteractiveNetworkDenyValue => "禁止",
        Message::InteractiveContextLabel => "上下文",
        Message::InteractiveTokensLabel => "令牌",
        Message::InteractiveTokensUnit => "令牌",
        Message::InteractiveMessagesUnit => "条消息",
        Message::InteractiveAllowPanelErrorPrefix => "权限面板错误：",
        Message::InteractiveGoalStatusPrefix => "目标：",
        Message::InteractiveGoalStatePrefix => "状态：",
        Message::InteractiveGoalStateRunning => "运行中",
        Message::InteractiveGoalStatePaused => "已暂停",
        Message::InteractiveGoalStateNeedsUser => "等待用户",
        Message::InteractiveGoalStateFailed => "执行失败",
        Message::InteractiveGoalResumeStarted => "正在恢复目标：",
        Message::InteractiveGoalNoResumable => "当前会话没有可恢复的目标。",
        Message::InteractiveWorkspaceStatusPrefix => "工作区：",
        Message::InteractiveCdUsage => "用法：/cd PATH · /cd - 返回上一个工作区",
        Message::InteractiveCompactTitle => "上下文压缩",
        Message::InteractiveCompactNoActiveSession => "当前没有活动会话",
        Message::InteractiveCompactAlready => "已经完成压缩",
        Message::InteractiveCompactCompleted => "压缩完成",
        Message::InteractiveCompactSummarized => "已摘要",
        Message::InteractiveCompactKeptRecent => "保留最近",
        Message::InteractiveCompactSaved => "节省",
        Message::InteractiveCompactHistoryPreserved => "完整对话仍会保留用于回放",
        Message::InteractiveCompactFailedPrefix => "上下文压缩失败：",
        Message::InteractiveNewSessionTitle => "新会话",
        Message::InteractiveNewSessionPending => "下一条消息将开始全新会话",
        Message::InteractiveJobsTitle => "后台任务",
        Message::InteractiveJobsUsage => "用法：/jobs · /jobs ID · /jobs status ID · /jobs stop ID",
        Message::InteractiveUnknownCommandHint => "未知命令；输入 /help 查看可用命令",
        Message::InteractiveChoiceLabel => "请选择",
        Message::InteractiveModelStatusPrefix => "模型：",
        Message::InteractiveDebugStatusPrefix => "调试：",
        Message::InteractiveAllowStatusPrefix => "权限：",
        Message::InteractiveDefaultModelLabel => "(默认)",
        Message::InteractiveTurnsLabel => "轮次",
        Message::InteractiveToolsLabel => "工具",
        Message::InteractiveNetworkLabel => "网络",
        Message::InteractiveInputPlaceholder => "输入消息…",
        Message::InteractiveExecutionDraftPrefix => "↳",
        Message::InteractiveExecutionQueuedPrefix => "已排队补充指令：",
        Message::InteractiveExecutionInterruptHint => "Esc 中断当前执行",
        Message::InteractiveExecutionInputHint => "Esc 中断 · Enter 排队补充指令",
        Message::AgentStatusThinking => "正在思考",
        Message::AgentStatusRetryingModel => "正在重试模型响应",
        Message::AgentStatusStrictToolStrategy => "严格工具策略",
        Message::AgentStatusChooseFromAvailableTools => "可用工具中选取",
        Message::AgentStatusReadyToContinue => "已就绪，等待下一步",
        Message::AgentStatusEnteredMode => "已进入模式",
        Message::AgentStatusReplyingDirectly => "准备直接回复",
        Message::AgentStatusExecutingPlan => "正在执行计划",
        Message::AgentStatusWaitingForUserConfirmation => "等待用户确认",
        Message::AgentStatusProcessing => "处理中",
        Message::AgentStatusPreparing => "正在准备",
        Message::AgentStatusModelUnavailable => "模型暂时不可用",
        Message::AgentDoneLabel => "完成",
        Message::AgentErrorLabel => "错误",
        Message::AgentToolLabel => "工具",
        Message::AgentAgentLabel => "代理",
        Message::AgentSessionLabel => "会话",
        Message::AgentStatusLabel => "状态",
        Message::AgentInfoLabel => "信息",
        Message::AgentReadyCurrentModePrefix => "已就绪，当前为",
        Message::AgentReadyCurrentModeSuffix => "模式",
        Message::AgentStatusDetailsSeparator => "、",
        Message::AgentPreparingToExecutePrefix => "准备执行",
        Message::AgentActionCountSuffixSingle => "个动作",
        Message::AgentActionCountSuffixPlural => "个动作",
        Message::AgentTurnPrefix => "第",
        Message::AgentTurnSuffix => "轮",
        Message::AgentInspectingProject => "正在查看项目",
        Message::AgentEditingFiles => "正在修改文件",
        Message::AgentRunningCommand => "正在执行命令",
        Message::AgentAccessingNetwork => "正在访问网络",
        Message::AgentSearchingTools => "正在查找工具",
        Message::AgentStartingSubtask => "正在启动子任务",
        Message::AgentFileModified => "已修改文件",
        Message::AgentCommandCompleted => "命令已完成",
        Message::AgentNetworkRequestCompleted => "网络请求已完成",
        Message::AgentSubtaskCompleted => "子任务已完成",
        Message::AgentFailedToModifyFile => "无法修改文件",
        Message::AgentCommandFailed => "命令执行失败",
        Message::AgentNetworkRequestFailed => "网络请求失败",
        Message::AgentSubtaskFailed => "子任务失败",
        Message::AgentOperationFailed => "操作失败",
        Message::InteractiveUnknownPath => "(未知)",
        Message::InteractiveStatusOn => "开启",
        Message::InteractiveStatusOff => "关闭",
        Message::InteractiveInvalidLanguageValue => "不支持的语言",
        Message::InvalidPortValue => "--port 必须是 1~65535 之间的数字",
        Message::UnknownAgentModePrefix => "未知 --agent-mode",
        Message::NetworkPolicyMustBeOneOf => "--network-policy 只能是 allow、ask、deny 之一",
        Message::ForceToolSetMustBeOneOf => "--force-tool-set 只能是 auto、all、write 之一",
        Message::FailedReadKeyEventPrefix => "读取按键事件失败：",
        Message::FailedFlushPromptPrefix => "刷新输入提示失败：",
        Message::InteractiveRawModeFailedPrefix => "开启原始输入模式失败：",
    }
}

fn de(message: Message) -> &'static str {
    match message {
        Message::ErrorPrefix => "Fehler:",
        Message::NoSavedSessions => "Keine gespeicherten Sitzungen.",
        Message::MessagesUnit => "Nachrichten",
        Message::SessionLabel => "Sitzung:",
        Message::CreatedLabel => "erstellt:",
        Message::UpdatedLabel => "aktualisiert:",
        Message::MessagesLabel => "Nachrichten:",
        Message::DeletedSessionPrefix => "Gelöschte Sitzung",
        Message::FailedReadPromptPrefix => "Prompt konnte nicht gelesen werden:",
        Message::PromptRequired => "Nachricht erforderlich",
        Message::SessionRequiresAction => "session benötigt eine Aktion: show oder delete",
        Message::RequiresSessionIdSuffix => "benötigt eine Sitzungs-ID",
        Message::UnknownSessionActionPrefix => "unbekannte session-Aktion:",
        Message::UnknownCommandPrefix => "unbekannter Befehl:",
        Message::MaxIterationsPositive => "--max-iterations muss eine nichtnegative Ganzzahl sein",
        Message::UnknownOptionPrefix => "unbekannte Option:",
        Message::UnexpectedSessionArgs => "unerwartete Zusatzargumente für session-Befehl",
        Message::MissingEndpoint => "--endpoint oder llm.endpoint in einer Nola-Konfigurationsdatei fehlt",
        Message::ContinueOrSession => "verwende entweder --continue oder --session, nicht beides",
        Message::CurrentDirFailedPrefix => "current_dir fehlgeschlagen:",
        Message::RequiresValueSuffix => "benötigt einen Wert",
        Message::FailedResolvePathPrefix => "Pfadauflösung fehlgeschlagen",
        Message::ActiveSessionLabel => "Sitzung:",
        Message::FailedCreateSessionDirPrefix => {
            "Sitzungsverzeichnis konnte nicht erstellt werden:"
        }
        Message::FailedReadSessionPrefix => "Sitzung konnte nicht gelesen werden",
        Message::FailedParseSessionPrefix => "Sitzung konnte nicht geparst werden",
        Message::FailedSerializeSessionPrefix => "Sitzung konnte nicht serialisiert werden:",
        Message::FailedSaveSessionPrefix => "Sitzung konnte nicht gespeichert werden:",
        Message::FailedReadSessionsPrefix => "Sitzungen konnten nicht gelesen werden:",
        Message::FailedReadSessionEntryPrefix => "Sitzungseintrag konnte nicht gelesen werden:",
        Message::FailedDeleteSessionPrefix => "Sitzung konnte nicht gelöscht werden",
        Message::HomeNotSet => "HOME ist nicht gesetzt; Sitzungen können nicht gespeichert werden",
        Message::FailedReadLatestSessionPrefix => "letzte Sitzung konnte nicht gelesen werden:",
        Message::FailedSaveLatestSessionPrefix => "letzte Sitzung konnte nicht gespeichert werden:",
        Message::InvalidSessionId => {
            "Sitzungs-ID darf nur Buchstaben, Zahlen, '-' und '_' enthalten"
        }
        Message::FailedCreateHttpClientPrefix => "HTTP-Client konnte nicht erstellt werden:",
        Message::LlmRequestFailedPrefix => "LLM-Anfrage fehlgeschlagen",
        Message::LlmStreamReadFailedPrefix => "LLM-Stream konnte nicht gelesen werden:",
        Message::ToolRequiresApprovalPrefix => "Tool",
        Message::ToolRequiresApprovalSuffix => "benötigt Freigabe.",
        Message::ArgsLabel => "Argumente:",
        Message::AllowOncePrompt => "Einmal erlauben? [y/N] ",
        Message::PermissionDeniedByUser => "Vom Benutzer abgelehnt",
        Message::PermissionDeniedGuidance => {
            "Berechtigung abgelehnt; Aktion nicht ausgeführt. Mit /allow Berechtigungen prüfen. Explizite deny-Regeln in der Konfiguration ändern und erneut beauftragen. Systemberechtigungen gelten weiterhin."
        }
        Message::SubAgentLabel => "Sub-Agent:",
        Message::TaskLabel => "Aufgabe:",
        Message::CapabilityLabel => "Fähigkeit:",
        Message::PromptLabel => "Prompt:",
        Message::DelegateAgentCapability => {
            "darf Befehle ausführen oder Dateien innerhalb der konfigurierten Berechtigungen ändern"
        }
        Message::DelegateAgentConfirmationRequired => {
            "Ausdrückliche Sub-Agent-Freigabe erforderlich; jedes Tool unterliegt weiterhin seiner Ressourcenberechtigung."
        }
        Message::NetworkAskNonInteractive => {
            "Netzwerkanfrage blockiert (policy=ask) im nicht interaktiven Modus. Verwende --network-policy=allow, um fortzufahren."
        }
        Message::NetworkAskPrompt => "Modell-Netzwerkanfrage jetzt erlauben? [y/N]: ",
        Message::NetworkAllowedByUser => "Netzwerkanfrage für Modellaufruf vom Benutzer erlaubt.",
        Message::NetworkDeniedByUser => "Modell-Netzwerkanfrage vom Benutzer abgelehnt.",
        Message::DisplayReadPrefix => "Gelesen",
        Message::DisplayWrotePrefix => "Geschrieben",
        Message::DisplayEditedPrefix => "Bearbeitet",
        Message::DisplayListedPrefix => "Aufgelistet",
        Message::EntriesUnit => "Einträge",
        Message::MatchesUnit => "Treffer",
        Message::ReachedMaxIterationsPrefix => "\n\nMaximale Rundenzahl erreicht (",
        Message::ReachedMaxIterationsSuffix => "); aktuelles Ergebnis wird zurückgegeben.",
        Message::InteractiveQueuedRequest => "Anfrage in der Warteschlange:",
        Message::InteractiveRunning => "laufen...",
        Message::InteractiveReadyForNextRequest => "bereit für die nächste Anfrage",
        Message::InteractiveModelLabel => "Modell",
        Message::InteractiveParamsLabel => "Parameter",
        Message::InteractiveWorkingDirLabel => "Arbeitsrichtung",
        Message::InteractiveWorkspaceLabel => "Arbeitsbereich",
        Message::InteractiveProviderLabel => "Anbieter",
        Message::InteractiveProviderLocal => "lokale Konfiguration",
        Message::InteractiveModeFieldLabel => "Modus",
        Message::InteractiveNetworkFieldLabel => "Netzwerk",
        Message::InteractiveEndpointLabel => "Endpunkt",
        Message::InteractiveAvailableModels => "Verfügbare Modelle:",
        Message::InteractiveModelUsageHint => "Verwendung: /model <Name>",
        Message::InteractiveAvailableCommands => "Verfügbare Befehle:",
        Message::InteractiveSlashHelpHint => "Hilfe anzeigen",
        Message::InteractiveSlashModelHint => "Modelle auflisten, /model <Name> zum Wechseln",
        Message::InteractiveSlashExecutorHint => "Kernausführungseinheit auflisten oder wechseln",
        Message::InteractiveAvailableExecutors => "Ausführungseinheiten:",
        Message::InteractiveExecutorUsage => "Verwendung: /executor <nola|codex|claude_code|opencode>",
        Message::InteractiveExecutorSelected => "Ausführungseinheit ausgewählt:",
        Message::InteractiveExecutorUnavailable => "Ausführungseinheit nicht verfügbar:",
        Message::InteractiveExecutorLabel => "Ausführung",
        Message::InteractiveSlashDebugHint => "Debug-Details anzeigen oder umschalten",
        Message::InteractiveSlashDebugUsage => "Verwendung: /debug am | /debug aus",
        Message::InteractiveSlashAllowHint => "Berechtigungsmodus festlegen",
        Message::InteractiveSlashAllowUsage => "Verwendung: /allow – Berechtigungsauswahlfeld öffnen",
        Message::InteractiveAllowPanelTitle => "Berechtigungsmodus",
        Message::InteractiveAllowPanelCautious => "Vorsichtig",
        Message::InteractiveAllowPanelCautiousDesc => "Löschen, Überschreiben, sensible Daten, ausgehende Inhalte und neue Verbindungen bestätigen. Normales Lesen und neue Dateien sind erlaubt.",
        Message::InteractiveAllowPanelAllowAll => "Alles erlauben",
        Message::InteractiveAllowPanelAllowAllDesc => "Keine Nola-Bestätigungen in diesem Gespräch. Systemberechtigungen gelten weiterhin.",
        Message::InteractiveAllowPanelSelected => "Ausgewählt:",
        Message::InteractiveAllowPanelCancelled => "Abgesagt",
        Message::InteractiveSlashGoalHint => "Arbeiten Sie selbstständig auf ein Ziel hin",
        Message::InteractiveSlashGoalUsage => "Verwendung: /goal <Ziel> [-c/--constraints <Einschränkungen>] | /goal Status | /goal Lebenslauf | /goal aus",
        Message::InteractiveSlashInitHint => "NOLA.md-Projektspeicher generieren (--dry-run, --force, --output PATH)",
        Message::InteractiveSlashCdHint => "Arbeitsbereich wechseln und eine neue Projektsitzung starten (/cd – zurück)",
        Message::InteractiveSlashCompactHint => "Kompakter Modellkontext unter Beibehaltung des vollständigen Wiedergabeverlaufs",
        Message::InteractiveSlashNewHint => "Starten Sie eine saubere Sitzung im aktuellen Arbeitsbereich",
        Message::InteractiveSlashPlanHint => "Zeigt den aktuellen Sitzungsausführungsplan an",
        Message::InteractiveSlashJobsHint => "Persistente Hintergrundaufgaben auflisten, prüfen oder stoppen",
        Message::InteractiveSlashResumeHint => "Öffnen Sie die Seite zur Sitzungswiederherstellung",
        Message::InteractiveSlashContinueHint => "Setzen Sie die letzte Projektsitzung fort",
        Message::InteractiveSlashHistoryHint => "Vollständigen Sitzungsverlauf erneut drucken",
        Message::InteractiveSlashStatusHint => "Aktuellen Laufzeitstatus anzeigen",
        Message::InteractiveSlashExitHint => "Beenden Sie die aktuelle Sitzung",
        Message::InteractiveSlashQuitHint => "q/quit/exit wird ebenfalls beendet",
        Message::InteractiveWelcomeTagline => "örtlicher erster Assistent",
        Message::InteractiveWelcomeHint => "Geben Sie /help für Befehle ein. Verwenden Sie /model, um das Modell zu wechseln",
        Message::InteractiveAutoValue => "auto",
        Message::InteractiveNetworkAllowValue => "erlauben",
        Message::InteractiveNetworkAskValue => "fragen",
        Message::InteractiveNetworkDenyValue => "leugnen",
        Message::InteractiveContextLabel => "Kontext",
        Message::InteractiveTokensLabel => "Token",
        Message::InteractiveTokensUnit => "Token",
        Message::InteractiveMessagesUnit => "Nachrichten",
        Message::InteractiveAllowPanelErrorPrefix => "Fehler im Berechtigungsfeld:",
        Message::InteractiveGoalStatusPrefix => "Ziel:",
        Message::InteractiveGoalStatePrefix => "Zustand:",
        Message::InteractiveGoalStateRunning => "laufen",
        Message::InteractiveGoalStatePaused => "pausierte",
        Message::InteractiveGoalStateNeedsUser => "Warten auf Benutzer",
        Message::InteractiveGoalStateFailed => "gescheitert",
        Message::InteractiveGoalResumeStarted => "Fortlaufendes Ziel:",
        Message::InteractiveGoalNoResumable => "In dieser Sitzung ist kein fortsetzbares Ziel verfügbar.",
        Message::InteractiveWorkspaceStatusPrefix => "Arbeitsbereich:",
        Message::InteractiveCdUsage => "Verwendung: /cd PATH · /cd – kehrt zum vorherigen Arbeitsbereich zurück",
        Message::InteractiveCompactTitle => "Kontextverdichtung",
        Message::InteractiveCompactNoActiveSession => "Keine aktive Sitzung",
        Message::InteractiveCompactAlready => "schon kompakt",
        Message::InteractiveCompactCompleted => "verdichtet",
        Message::InteractiveCompactSummarized => "zusammengefasst",
        Message::InteractiveCompactKeptRecent => "aktuell gehalten",
        Message::InteractiveCompactSaved => "gespeichert",
        Message::InteractiveCompactHistoryPreserved => "Das vollständige Transkript bleibt zur Wiedergabe erhalten",
        Message::InteractiveCompactFailedPrefix => "Kontextkomprimierung fehlgeschlagen:",
        Message::InteractiveNewSessionTitle => "neue Sitzung",
        Message::InteractiveNewSessionPending => "Eine saubere Sitzung beginnt mit der nächsten Nachricht",
        Message::InteractiveJobsTitle => "Hintergrundaufgaben",
        Message::InteractiveJobsUsage => "Verwendung: /jobs · /jobs ID · /jobs Status ID · /jobs Stopp ID",
        Message::InteractiveUnknownCommandHint => "Unbekannter Befehl; Geben Sie /help ein, um die verfügbaren Befehle anzuzeigen",
        Message::InteractiveChoiceLabel => "Bitte wählen Sie",
        Message::InteractiveModelStatusPrefix => "Modell:",
        Message::InteractiveDebugStatusPrefix => "debuggen:",
        Message::InteractiveAllowStatusPrefix => "erlauben:",
        Message::InteractiveDefaultModelLabel => "(Standard)",
        Message::InteractiveTurnsLabel => "dreht sich",
        Message::InteractiveToolsLabel => "Werkzeuge",
        Message::InteractiveNetworkLabel => "Netzwerk",
        Message::InteractiveUnknownPath => "(unbekannt)",
        Message::InteractiveStatusOn => "auf",
        Message::InteractiveStatusOff => "aus",
        Message::InteractiveInputPlaceholder => "Geben Sie eine Nachricht ein…",
        Message::InteractiveExecutionDraftPrefix => "↳",
        Message::InteractiveExecutionQueuedPrefix => "Anweisungen zur Weiterverfolgung in der Warteschlange:",
        Message::InteractiveExecutionInterruptHint => "Esc unterbricht den aktuellen Lauf",
        Message::InteractiveExecutionInputHint => "Esc abbrechen · Enter einreihen",
        Message::AgentStatusThinking => "Denken",
        Message::AgentStatusRetryingModel => "Modellantwort wird erneut versucht",
        Message::AgentStatusStrictToolStrategy => "strikte Werkzeugstrategie",
        Message::AgentStatusChooseFromAvailableTools => "Wählen Sie aus den verfügbaren Tools",
        Message::AgentStatusReadyToContinue => "bereit, weiterzumachen",
        Message::AgentStatusEnteredMode => "Modus eingegeben",
        Message::AgentStatusReplyingDirectly => "direkt antworten",
        Message::AgentStatusExecutingPlan => "Plan ausführen",
        Message::AgentStatusWaitingForUserConfirmation => "Warten auf die Bestätigung des Benutzers",
        Message::AgentStatusProcessing => "Verarbeitung",
        Message::AgentStatusPreparing => "vorbereiten",
        Message::AgentStatusModelUnavailable => "Modell vorübergehend nicht verfügbar",
        Message::AgentDoneLabel => "erledigt",
        Message::AgentErrorLabel => "Fehler",
        Message::AgentToolLabel => "Werkzeug",
        Message::AgentAgentLabel => "Agent",
        Message::AgentSessionLabel => "Sitzung",
        Message::AgentStatusLabel => "Status",
        Message::AgentInfoLabel => "Infos",
        Message::AgentReadyCurrentModePrefix => "Bereit, aktueller Modus ist",
        Message::AgentReadyCurrentModeSuffix => "",
        Message::AgentStatusDetailsSeparator => ",",
        Message::AgentPreparingToExecutePrefix => "Vorbereitung zur Ausführung",
        Message::AgentActionCountSuffixSingle => "Aktion",
        Message::AgentActionCountSuffixPlural => "Aktionen",
        Message::AgentTurnPrefix => "drehen",
        Message::AgentTurnSuffix => "",
        Message::AgentInspectingProject => "Inspektion des Projekts",
        Message::AgentEditingFiles => "Bearbeiten von Dateien",
        Message::AgentRunningCommand => "laufenden Befehl",
        Message::AgentAccessingNetwork => "auf das Netzwerk zugreifen",
        Message::AgentSearchingTools => "Suchwerkzeuge",
        Message::AgentStartingSubtask => "Teilaufgabe starten",
        Message::AgentFileModified => "Datei geändert",
        Message::AgentCommandCompleted => "Befehl abgeschlossen",
        Message::AgentNetworkRequestCompleted => "Netzwerkanfrage abgeschlossen",
        Message::AgentSubtaskCompleted => "Teilaufgabe erledigt",
        Message::AgentFailedToModifyFile => "Datei konnte nicht geändert werden",
        Message::AgentCommandFailed => "Befehl fehlgeschlagen",
        Message::AgentNetworkRequestFailed => "Netzwerkanforderung fehlgeschlagen",
        Message::AgentSubtaskFailed => "Unteraufgabe fehlgeschlagen",
        Message::AgentOperationFailed => "Der Vorgang ist fehlgeschlagen",
        Message::InteractiveInvalidLanguageValue => "nicht unterstützte Sprache",
        Message::InvalidPortValue => "--port muss eine Zahl zwischen 1 und 65535 sein",
        Message::UnknownAgentModePrefix => "unbekannt --agent-mode",
        Message::NetworkPolicyMustBeOneOf => "--network-policy muss eines von sein: erlauben, fragen, verweigern",
        Message::ForceToolSetMustBeOneOf => "--force-tool-set muss einer der folgenden Werte sein: auto, all, write",
        Message::FailedReadKeyEventPrefix => "Schlüsselereignis konnte nicht gelesen werden:",
        Message::FailedFlushPromptPrefix => "Eingabeaufforderung konnte nicht geleert werden:",
        Message::InteractiveRawModeFailedPrefix => "Der Raw-Modus konnte nicht aktiviert werden:",
    }
}

fn es(message: Message) -> &'static str {
    match message {
        Message::ErrorPrefix => "error:",
        Message::NoSavedSessions => "No hay sesiones guardadas.",
        Message::MessagesUnit => "mensajes",
        Message::SessionLabel => "sesión:",
        Message::CreatedLabel => "creada:",
        Message::UpdatedLabel => "actualizada:",
        Message::MessagesLabel => "mensajes:",
        Message::DeletedSessionPrefix => "Sesión eliminada",
        Message::FailedReadPromptPrefix => "no se pudo leer el prompt:",
        Message::PromptRequired => "se requiere un mensaje",
        Message::SessionRequiresAction => "session requiere una acción: show o delete",
        Message::RequiresSessionIdSuffix => "requiere un ID de sesión",
        Message::UnknownSessionActionPrefix => "acción de session desconocida:",
        Message::UnknownCommandPrefix => "comando desconocido:",
        Message::MaxIterationsPositive => "--max-iterations debe ser un entero no negativo",
        Message::UnknownOptionPrefix => "opción desconocida:",
        Message::UnexpectedSessionArgs => "argumentos extra inesperados para el comando session",
        Message::MissingEndpoint => "falta --endpoint o llm.endpoint en un archivo de configuración de Nola",
        Message::ContinueOrSession => "usa --continue o --session, no ambos",
        Message::CurrentDirFailedPrefix => "current_dir falló:",
        Message::RequiresValueSuffix => "requiere un valor",
        Message::FailedResolvePathPrefix => "no se pudo resolver",
        Message::ActiveSessionLabel => "sesión:",
        Message::FailedCreateSessionDirPrefix => "no se pudo crear el directorio de sesiones:",
        Message::FailedReadSessionPrefix => "no se pudo leer la sesión",
        Message::FailedParseSessionPrefix => "no se pudo analizar la sesión",
        Message::FailedSerializeSessionPrefix => "no se pudo serializar la sesión:",
        Message::FailedSaveSessionPrefix => "no se pudo guardar la sesión:",
        Message::FailedReadSessionsPrefix => "no se pudieron leer las sesiones:",
        Message::FailedReadSessionEntryPrefix => "no se pudo leer la entrada de sesión:",
        Message::FailedDeleteSessionPrefix => "no se pudo eliminar la sesión",
        Message::HomeNotSet => "HOME no está definido; no se pueden guardar sesiones",
        Message::FailedReadLatestSessionPrefix => "no se pudo leer la última sesión:",
        Message::FailedSaveLatestSessionPrefix => "no se pudo guardar la última sesión:",
        Message::InvalidSessionId => {
            "el ID de sesión solo puede contener letras, números, '-' y '_'"
        }
        Message::FailedCreateHttpClientPrefix => "no se pudo crear el cliente HTTP:",
        Message::LlmRequestFailedPrefix => "falló la solicitud LLM",
        Message::LlmStreamReadFailedPrefix => "falló la lectura del stream LLM:",
        Message::ToolRequiresApprovalPrefix => "La herramienta",
        Message::ToolRequiresApprovalSuffix => "requiere aprobación.",
        Message::ArgsLabel => "argumentos:",
        Message::AllowOncePrompt => "¿Permitir una vez? [y/N] ",
        Message::PermissionDeniedByUser => "Permiso denegado por el usuario",
        Message::PermissionDeniedGuidance => {
            "Permiso denegado; la acción no se ejecutó. Revisa permisos con /allow. Cambia las reglas deny explícitas en la configuración y solicita otro intento. Los permisos del sistema siguen vigentes."
        }
        Message::SubAgentLabel => "subagente:",
        Message::TaskLabel => "tarea:",
        Message::CapabilityLabel => "capacidad:",
        Message::PromptLabel => "prompt:",
        Message::DelegateAgentCapability => {
            "puede ejecutar comandos o modificar archivos dentro de los permisos configurados"
        }
        Message::DelegateAgentConfirmationRequired => {
            "Se requiere aprobación explícita del subagente; cada herramienta mantiene sus propios permisos de recursos."
        }
        Message::NetworkAskNonInteractive => {
            "Solicitud de red bloqueada (policy=ask) en modo no interactivo. Usa --network-policy=allow para continuar."
        }
        Message::NetworkAskPrompt => "¿Permitir ahora la solicitud de red del modelo? [y/N]: ",
        Message::NetworkAllowedByUser => "Solicitud de red del modelo permitida por el usuario.",
        Message::NetworkDeniedByUser => "Solicitud de red del modelo denegada por el usuario.",
        Message::DisplayReadPrefix => "Leído",
        Message::DisplayWrotePrefix => "Escrito",
        Message::DisplayEditedPrefix => "Editado",
        Message::DisplayListedPrefix => "Listado",
        Message::EntriesUnit => "entradas",
        Message::MatchesUnit => "coincidencias",
        Message::ReachedMaxIterationsPrefix => "\n\nSe alcanzó el máximo de iteraciones (",
        Message::ReachedMaxIterationsSuffix => "); se devuelve el resultado actual.",
        Message::InteractiveQueuedRequest => "solicitud en cola:",
        Message::InteractiveRunning => "corriendo...",
        Message::InteractiveReadyForNextRequest => "listo para la próxima solicitud",
        Message::InteractiveModelLabel => "modelo",
        Message::InteractiveParamsLabel => "parámetros",
        Message::InteractiveWorkingDirLabel => "Director de trabajo",
        Message::InteractiveWorkspaceLabel => "Espacio de trabajo",
        Message::InteractiveProviderLabel => "Proveedor",
        Message::InteractiveProviderLocal => "configuración local",
        Message::InteractiveModeFieldLabel => "Modo",
        Message::InteractiveNetworkFieldLabel => "Red",
        Message::InteractiveEndpointLabel => "Punto final",
        Message::InteractiveAvailableModels => "Modelos disponibles:",
        Message::InteractiveModelUsageHint => "Uso: /model <nombre>",
        Message::InteractiveAvailableCommands => "Comandos disponibles:",
        Message::InteractiveSlashHelpHint => "Mostrar ayuda",
        Message::InteractiveSlashModelHint => "Listar modelos, /model <nombre> para cambiar",
        Message::InteractiveSlashExecutorHint => "Enumerar o cambiar la unidad de ejecución principal",
        Message::InteractiveAvailableExecutors => "Unidades de ejecución:",
        Message::InteractiveExecutorUsage => "Uso: /executor <nola|codex|claude_code|opencode>",
        Message::InteractiveExecutorSelected => "Unidad de ejecución seleccionada:",
        Message::InteractiveExecutorUnavailable => "Unidad de ejecución no disponible:",
        Message::InteractiveExecutorLabel => "Ejecutor",
        Message::InteractiveSlashDebugHint => "Mostrar o alternar detalles de depuración",
        Message::InteractiveSlashDebugUsage => "Uso: /debug en | /debug desactivado",
        Message::InteractiveSlashAllowHint => "Establecer modo de permiso",
        Message::InteractiveSlashAllowUsage => "Uso: /allow — abrir panel de selección de permisos",
        Message::InteractiveAllowPanelTitle => "Modo de permiso",
        Message::InteractiveAllowPanelCautious => "Prudente",
        Message::InteractiveAllowPanelCautiousDesc => "Confirma borrados, sobrescrituras, datos sensibles, envíos y conexiones nuevas. Las lecturas normales y los archivos nuevos continúan.",
        Message::InteractiveAllowPanelAllowAll => "Permitir todo",
        Message::InteractiveAllowPanelAllowAllDesc => "Sin confirmaciones de Nola en esta conversación. Siguen vigentes los permisos del sistema.",
        Message::InteractiveAllowPanelSelected => "Seleccionado:",
        Message::InteractiveAllowPanelCancelled => "Cancelado",
        Message::InteractiveSlashGoalHint => "Trabajar de forma autónoma hacia una meta.",
        Message::InteractiveSlashGoalUsage => "Uso: /goal <objetivo> [-c/--constraints <restricciones>] | /goal estado | /goal currículum | /goal desactivado",
        Message::InteractiveSlashInitHint => "Generar memoria del proyecto NOLA.md (--dry-run, --force, --output RUTA)",
        Message::InteractiveSlashCdHint => "Cambie de espacio de trabajo e inicie una nueva sesión de proyecto (/cd - para regresar)",
        Message::InteractiveSlashCompactHint => "Contexto del modelo compacto al tiempo que se conserva el historial de reproducción completo",
        Message::InteractiveSlashNewHint => "Iniciar una sesión limpia en el espacio de trabajo actual",
        Message::InteractiveSlashPlanHint => "Mostrar el plan de ejecución de la sesión actual",
        Message::InteractiveSlashJobsHint => "Enumerar, inspeccionar o detener tareas persistentes en segundo plano",
        Message::InteractiveSlashResumeHint => "Abra la página de restauración de sesión",
        Message::InteractiveSlashContinueHint => "Continuar con la última sesión del proyecto",
        Message::InteractiveSlashHistoryHint => "Reimprimir el historial completo de la sesión",
        Message::InteractiveSlashStatusHint => "Mostrar el estado de ejecución actual",
        Message::InteractiveSlashExitHint => "Salir de la sesión actual",
        Message::InteractiveSlashQuitHint => "q/quit/exit también sale",
        Message::InteractiveWelcomeTagline => "primer asistente local",
        Message::InteractiveWelcomeHint => "Escriba /help para comandos · use /model para cambiar de modelo",
        Message::InteractiveAutoValue => "automático",
        Message::InteractiveNetworkAllowValue => "permitir",
        Message::InteractiveNetworkAskValue => "preguntar",
        Message::InteractiveNetworkDenyValue => "negar",
        Message::InteractiveContextLabel => "Contexto",
        Message::InteractiveTokensLabel => "fichas",
        Message::InteractiveTokensUnit => "fichas",
        Message::InteractiveMessagesUnit => "mensajes",
        Message::InteractiveAllowPanelErrorPrefix => "error del panel de permisos:",
        Message::InteractiveGoalStatusPrefix => "objetivo:",
        Message::InteractiveGoalStatePrefix => "estado:",
        Message::InteractiveGoalStateRunning => "corriendo",
        Message::InteractiveGoalStatePaused => "pausado",
        Message::InteractiveGoalStateNeedsUser => "esperando al usuario",
        Message::InteractiveGoalStateFailed => "falló",
        Message::InteractiveGoalResumeStarted => "reanudando el objetivo:",
        Message::InteractiveGoalNoResumable => "No hay ningún objetivo reanudable disponible en esta sesión.",
        Message::InteractiveWorkspaceStatusPrefix => "espacio de trabajo:",
        Message::InteractiveCdUsage => "Uso: /cd RUTA · /cd - regresa al espacio de trabajo anterior",
        Message::InteractiveCompactTitle => "compactación de contexto",
        Message::InteractiveCompactNoActiveSession => "ninguna sesión activa",
        Message::InteractiveCompactAlready => "ya compacto",
        Message::InteractiveCompactCompleted => "compactado",
        Message::InteractiveCompactSummarized => "resumido",
        Message::InteractiveCompactKeptRecent => "mantenido reciente",
        Message::InteractiveCompactSaved => "salvado",
        Message::InteractiveCompactHistoryPreserved => "La transcripción completa se conserva para su reproducción.",
        Message::InteractiveCompactFailedPrefix => "La compactación del contexto falló:",
        Message::InteractiveNewSessionTitle => "nueva sesión",
        Message::InteractiveNewSessionPending => "una sesión limpia comenzará con el siguiente mensaje",
        Message::InteractiveJobsTitle => "tareas en segundo plano",
        Message::InteractiveJobsUsage => "Uso: /jobs · /jobs ID · /jobs estado ID · /jobs detener ID",
        Message::InteractiveUnknownCommandHint => "Comando desconocido; escriba /help para ver los comandos disponibles",
        Message::InteractiveChoiceLabel => "Por favor elige",
        Message::InteractiveModelStatusPrefix => "modelo:",
        Message::InteractiveDebugStatusPrefix => "depurar:",
        Message::InteractiveAllowStatusPrefix => "permitir:",
        Message::InteractiveDefaultModelLabel => "(predeterminado)",
        Message::InteractiveTurnsLabel => "vueltas",
        Message::InteractiveToolsLabel => "herramientas",
        Message::InteractiveNetworkLabel => "red",
        Message::InteractiveUnknownPath => "(desconocido)",
        Message::InteractiveStatusOn => "en",
        Message::InteractiveStatusOff => "apagado",
        Message::InteractiveInputPlaceholder => "Escribe un mensaje…",
        Message::InteractiveExecutionDraftPrefix => "↳",
        Message::InteractiveExecutionQueuedPrefix => "Instrucciones de seguimiento en cola:",
        Message::InteractiveExecutionInterruptHint => "Esc interrumpe la ejecución actual",
        Message::InteractiveExecutionInputHint => "Esc interrumpir · Enter poner en cola",
        Message::AgentStatusThinking => "pensando",
        Message::AgentStatusRetryingModel => "reintentando la respuesta del modelo",
        Message::AgentStatusStrictToolStrategy => "estricta estrategia de herramientas",
        Message::AgentStatusChooseFromAvailableTools => "elegir entre las herramientas disponibles",
        Message::AgentStatusReadyToContinue => "listo para continuar",
        Message::AgentStatusEnteredMode => "modo ingresado",
        Message::AgentStatusReplyingDirectly => "respondiendo directamente",
        Message::AgentStatusExecutingPlan => "plan de ejecución",
        Message::AgentStatusWaitingForUserConfirmation => "esperando confirmación del usuario",
        Message::AgentStatusProcessing => "procesamiento",
        Message::AgentStatusPreparing => "preparando",
        Message::AgentStatusModelUnavailable => "modelo no disponible temporalmente",
        Message::AgentDoneLabel => "hecho",
        Message::AgentErrorLabel => "error",
        Message::AgentToolLabel => "herramienta",
        Message::AgentAgentLabel => "agente",
        Message::AgentSessionLabel => "sesión",
        Message::AgentStatusLabel => "estado",
        Message::AgentInfoLabel => "información",
        Message::AgentReadyCurrentModePrefix => "listo, el modo actual es",
        Message::AgentReadyCurrentModeSuffix => "",
        Message::AgentStatusDetailsSeparator => ",",
        Message::AgentPreparingToExecutePrefix => "preparándose para ejecutar",
        Message::AgentActionCountSuffixSingle => "acción",
        Message::AgentActionCountSuffixPlural => "acciones",
        Message::AgentTurnPrefix => "girar",
        Message::AgentTurnSuffix => "",
        Message::AgentInspectingProject => "inspeccionando el proyecto",
        Message::AgentEditingFiles => "editar archivos",
        Message::AgentRunningCommand => "comando en ejecución",
        Message::AgentAccessingNetwork => "accediendo a la red",
        Message::AgentSearchingTools => "herramientas de búsqueda",
        Message::AgentStartingSubtask => "subtarea inicial",
        Message::AgentFileModified => "archivo modificado",
        Message::AgentCommandCompleted => "comando completado",
        Message::AgentNetworkRequestCompleted => "solicitud de red completada",
        Message::AgentSubtaskCompleted => "subtarea completada",
        Message::AgentFailedToModifyFile => "no se pudo modificar el archivo",
        Message::AgentCommandFailed => "comando fallido",
        Message::AgentNetworkRequestFailed => "solicitud de red fallida",
        Message::AgentSubtaskFailed => "subtarea fallida",
        Message::AgentOperationFailed => "operación fallida",
        Message::InteractiveInvalidLanguageValue => "idioma no soportado",
        Message::InvalidPortValue => "--port debe ser un número entre 1 y 65535",
        Message::UnknownAgentModePrefix => "desconocido --agent-mode",
        Message::NetworkPolicyMustBeOneOf => "--network-policy debe ser uno de: permitir, preguntar, negar",
        Message::ForceToolSetMustBeOneOf => "--force-tool-set debe ser uno de: auto, todo, escribir",
        Message::FailedReadKeyEventPrefix => "no se pudo leer el evento clave:",
        Message::FailedFlushPromptPrefix => "No se pudo vaciar el mensaje:",
        Message::InteractiveRawModeFailedPrefix => "no se pudo habilitar el modo sin formato:",
    }
}

fn fr(message: Message) -> &'static str {
    match message {
        Message::ErrorPrefix => "erreur :",
        Message::NoSavedSessions => "Aucune session enregistrée.",
        Message::MessagesUnit => "messages",
        Message::SessionLabel => "session :",
        Message::CreatedLabel => "créée :",
        Message::UpdatedLabel => "mise à jour :",
        Message::MessagesLabel => "messages :",
        Message::DeletedSessionPrefix => "Session supprimée",
        Message::FailedReadPromptPrefix => "échec de lecture du prompt :",
        Message::PromptRequired => "un message est requis",
        Message::SessionRequiresAction => "session nécessite une action : show ou delete",
        Message::RequiresSessionIdSuffix => "nécessite un ID de session",
        Message::UnknownSessionActionPrefix => "action session inconnue :",
        Message::UnknownCommandPrefix => "commande inconnue :",
        Message::MaxIterationsPositive => "--max-iterations doit être un entier non négatif",
        Message::UnknownOptionPrefix => "option inconnue :",
        Message::UnexpectedSessionArgs => {
            "arguments supplémentaires inattendus pour la commande session"
        }
        Message::MissingEndpoint => "--endpoint ou llm.endpoint manque dans un fichier de configuration Nola",
        Message::ContinueOrSession => "utilisez --continue ou --session, pas les deux",
        Message::CurrentDirFailedPrefix => "current_dir a échoué :",
        Message::RequiresValueSuffix => "nécessite une valeur",
        Message::FailedResolvePathPrefix => "échec de résolution",
        Message::ActiveSessionLabel => "session :",
        Message::FailedCreateSessionDirPrefix => "échec de création du répertoire de sessions :",
        Message::FailedReadSessionPrefix => "échec de lecture de la session",
        Message::FailedParseSessionPrefix => "échec d'analyse de la session",
        Message::FailedSerializeSessionPrefix => "échec de sérialisation de la session :",
        Message::FailedSaveSessionPrefix => "échec d'enregistrement de la session :",
        Message::FailedReadSessionsPrefix => "échec de lecture des sessions :",
        Message::FailedReadSessionEntryPrefix => "échec de lecture d'une entrée de session :",
        Message::FailedDeleteSessionPrefix => "échec de suppression de la session",
        Message::HomeNotSet => "HOME n'est pas défini ; impossible d'enregistrer les sessions",
        Message::FailedReadLatestSessionPrefix => "échec de lecture de la dernière session :",
        Message::FailedSaveLatestSessionPrefix => "échec d'enregistrement de la dernière session :",
        Message::InvalidSessionId => {
            "l'ID de session ne peut contenir que des lettres, chiffres, '-' et '_'"
        }
        Message::FailedCreateHttpClientPrefix => "échec de création du client HTTP :",
        Message::LlmRequestFailedPrefix => "échec de la requête LLM",
        Message::LlmStreamReadFailedPrefix => "échec de lecture du flux LLM :",
        Message::ToolRequiresApprovalPrefix => "L'outil",
        Message::ToolRequiresApprovalSuffix => "nécessite une approbation.",
        Message::ArgsLabel => "arguments :",
        Message::AllowOncePrompt => "Autoriser une fois ? [y/N] ",
        Message::PermissionDeniedByUser => "Autorisation refusée par l'utilisateur",
        Message::PermissionDeniedGuidance => {
            "Autorisation refusée ; action non exécutée. Vérifiez les droits avec /allow. Modifiez les règles deny explicites dans la configuration, puis demandez un nouvel essai. Les autorisations système restent applicables."
        }
        Message::SubAgentLabel => "sous-agent :",
        Message::TaskLabel => "tâche :",
        Message::CapabilityLabel => "capacité :",
        Message::PromptLabel => "prompt :",
        Message::DelegateAgentCapability => {
            "peut exécuter des commandes ou modifier des fichiers dans les permissions configurées"
        }
        Message::DelegateAgentConfirmationRequired => {
            "Une approbation explicite du sous-agent est requise ; chaque outil reste soumis à ses propres droits sur les ressources."
        }
        Message::NetworkAskNonInteractive => {
            "Requête réseau bloquée (policy=ask) en mode non interactif. Utilisez --network-policy=allow pour continuer."
        }
        Message::NetworkAskPrompt => "Autoriser maintenant la requête réseau du modèle ? [y/N] : ",
        Message::NetworkAllowedByUser => "Requête réseau du modèle autorisée par l'utilisateur.",
        Message::NetworkDeniedByUser => "Requête réseau du modèle refusée par l'utilisateur.",
        Message::DisplayReadPrefix => "Lu",
        Message::DisplayWrotePrefix => "Écrit",
        Message::DisplayEditedPrefix => "Modifié",
        Message::DisplayListedPrefix => "Listé",
        Message::EntriesUnit => "entrées",
        Message::MatchesUnit => "correspondances",
        Message::ReachedMaxIterationsPrefix => "\n\nNombre maximal d'itérations atteint (",
        Message::ReachedMaxIterationsSuffix => ") ; retour du résultat actuel.",
        Message::InteractiveQueuedRequest => "requête en file d'attente :",
        Message::InteractiveRunning => "courir...",
        Message::InteractiveReadyForNextRequest => "prêt pour la prochaine demande",
        Message::InteractiveModelLabel => "Modèle",
        Message::InteractiveParamsLabel => "Paramètres",
        Message::InteractiveWorkingDirLabel => "Répertoire de travail",
        Message::InteractiveWorkspaceLabel => "Espace de travail",
        Message::InteractiveProviderLabel => "Fournisseur",
        Message::InteractiveProviderLocal => "configuration locale",
        Message::InteractiveModeFieldLabel => "Mode",
        Message::InteractiveNetworkFieldLabel => "Réseau",
        Message::InteractiveEndpointLabel => "Point de terminaison",
        Message::InteractiveAvailableModels => "Modèles disponibles :",
        Message::InteractiveModelUsageHint => "Utilisation : /model <nom>",
        Message::InteractiveAvailableCommands => "Commandes disponibles :",
        Message::InteractiveSlashHelpHint => "Afficher l'aide",
        Message::InteractiveSlashModelHint => "Liste des modèles, /model <nom> pour changer",
        Message::InteractiveSlashExecutorHint => "Répertorier ou changer l’unité d’exécution principale",
        Message::InteractiveAvailableExecutors => "Unités d’exécution :",
        Message::InteractiveExecutorUsage => "Utilisation : /executor <nola|codex|claude_code|opencode>",
        Message::InteractiveExecutorSelected => "Unité d’exécution sélectionnée :",
        Message::InteractiveExecutorUnavailable => "Unité d’exécution indisponible :",
        Message::InteractiveExecutorLabel => "Exécuteur",
        Message::InteractiveSlashDebugHint => "Afficher ou basculer les détails du débogage",
        Message::InteractiveSlashDebugUsage => "Utilisation : /debug sur | /debug désactivé",
        Message::InteractiveSlashAllowHint => "Définir le mode d'autorisation",
        Message::InteractiveSlashAllowUsage => "Utilisation : /allow — ouvrir le panneau de sélection des autorisations",
        Message::InteractiveAllowPanelTitle => "Mode autorisation",
        Message::InteractiveAllowPanelCautious => "Prudent",
        Message::InteractiveAllowPanelCautiousDesc => "Confirmer suppressions, écrasements, accès sensibles, envois et nouvelles connexions. Les lectures ordinaires et créations continuent.",
        Message::InteractiveAllowPanelAllowAll => "Tout autoriser",
        Message::InteractiveAllowPanelAllowAllDesc => "Aucune confirmation Nola dans cette conversation. Les autorisations système restent actives.",
        Message::InteractiveAllowPanelSelected => "Sélectionné :",
        Message::InteractiveAllowPanelCancelled => "Annulé",
        Message::InteractiveSlashGoalHint => "Travailler de manière autonome vers un objectif",
        Message::InteractiveSlashGoalUsage => "Utilisation : /goal <objectif> [-c/--constraints <contraintes>] | /goal état | /goal reprendre | /goal désactivé",
        Message::InteractiveSlashInitHint => "Générer la mémoire du projet NOLA.md (--dry-run, --force, --output PATH)",
        Message::InteractiveSlashCdHint => "Changez d'espace de travail et démarrez une nouvelle session de projet (/cd - pour revenir)",
        Message::InteractiveSlashCompactHint => "Contexte de modèle compact tout en préservant l'historique complet des relectures",
        Message::InteractiveSlashNewHint => "Démarrer une session propre dans l'espace de travail actuel",
        Message::InteractiveSlashPlanHint => "Afficher le plan d'exécution de la session en cours",
        Message::InteractiveSlashJobsHint => "Répertorier, inspecter ou arrêter les tâches en arrière-plan persistantes",
        Message::InteractiveSlashResumeHint => "Ouvrir la page de restauration de session",
        Message::InteractiveSlashContinueHint => "Continuer la dernière session de projet",
        Message::InteractiveSlashHistoryHint => "Réimprimer l'historique complet de la session",
        Message::InteractiveSlashStatusHint => "Afficher l'état d'exécution actuel",
        Message::InteractiveSlashExitHint => "Quitter la session en cours",
        Message::InteractiveSlashQuitHint => "q/quit/exit se ferme également",
        Message::InteractiveWelcomeTagline => "premier assistant local",
        Message::InteractiveWelcomeHint => "Tapez /help pour les commandes · utilisez /model pour changer de modèle",
        Message::InteractiveAutoValue => "automobile",
        Message::InteractiveNetworkAllowValue => "permettre",
        Message::InteractiveNetworkAskValue => "demander",
        Message::InteractiveNetworkDenyValue => "nier",
        Message::InteractiveContextLabel => "Contexte",
        Message::InteractiveTokensLabel => "jetons",
        Message::InteractiveTokensUnit => "jetons",
        Message::InteractiveMessagesUnit => "messages",
        Message::InteractiveAllowPanelErrorPrefix => "erreur du panneau d'autorisation :",
        Message::InteractiveGoalStatusPrefix => "objectif :",
        Message::InteractiveGoalStatePrefix => "état :",
        Message::InteractiveGoalStateRunning => "courir",
        Message::InteractiveGoalStatePaused => "en pause",
        Message::InteractiveGoalStateNeedsUser => "en attente de l'utilisateur",
        Message::InteractiveGoalStateFailed => "échoué",
        Message::InteractiveGoalResumeStarted => "objectif de reprise :",
        Message::InteractiveGoalNoResumable => "Aucun objectif pouvant être repris n'est disponible dans cette session.",
        Message::InteractiveWorkspaceStatusPrefix => "espace de travail :",
        Message::InteractiveCdUsage => "Utilisation : /cd PATH · /cd - revient à l'espace de travail précédent",
        Message::InteractiveCompactTitle => "compactage du contexte",
        Message::InteractiveCompactNoActiveSession => "aucune session active",
        Message::InteractiveCompactAlready => "déjà compact",
        Message::InteractiveCompactCompleted => "compacté",
        Message::InteractiveCompactSummarized => "résumé",
        Message::InteractiveCompactKeptRecent => "gardé récent",
        Message::InteractiveCompactSaved => "enregistré",
        Message::InteractiveCompactHistoryPreserved => "la transcription complète est conservée pour la relecture",
        Message::InteractiveCompactFailedPrefix => "Le compactage du contexte a échoué :",
        Message::InteractiveNewSessionTitle => "nouvelle séance",
        Message::InteractiveNewSessionPending => "une session propre démarrera avec le prochain message",
        Message::InteractiveJobsTitle => "tâches en arrière-plan",
        Message::InteractiveJobsUsage => "Utilisation : /jobs · /jobs ID · /jobs état ID · /jobs arrêter ID",
        Message::InteractiveUnknownCommandHint => "Commande inconnue ; tapez /help pour voir les commandes disponibles",
        Message::InteractiveChoiceLabel => "Veuillez choisir",
        Message::InteractiveModelStatusPrefix => "modèle :",
        Message::InteractiveDebugStatusPrefix => "débogage :",
        Message::InteractiveAllowStatusPrefix => "permettre :",
        Message::InteractiveDefaultModelLabel => "(par défaut)",
        Message::InteractiveTurnsLabel => "tourne",
        Message::InteractiveToolsLabel => "outils",
        Message::InteractiveNetworkLabel => "réseau",
        Message::InteractiveUnknownPath => "(inconnu)",
        Message::InteractiveStatusOn => "sur",
        Message::InteractiveStatusOff => "éteint",
        Message::InteractiveInputPlaceholder => "Tapez un message…",
        Message::InteractiveExecutionDraftPrefix => "↳",
        Message::InteractiveExecutionQueuedPrefix => "Instructions de suivi en file d'attente :",
        Message::InteractiveExecutionInterruptHint => "Esc interrompt l'exécution en cours",
        Message::InteractiveExecutionInputHint => "Esc interrompre · Enter mettre en file",
        Message::AgentStatusThinking => "en pensant",
        Message::AgentStatusRetryingModel => "nouvelle tentative de réponse du modèle",
        Message::AgentStatusStrictToolStrategy => "stratégie d'outils stricte",
        Message::AgentStatusChooseFromAvailableTools => "choisir parmi les outils disponibles",
        Message::AgentStatusReadyToContinue => "prêt à continuer",
        Message::AgentStatusEnteredMode => "mode entré",
        Message::AgentStatusReplyingDirectly => "répondre directement",
        Message::AgentStatusExecutingPlan => "plan d'exécution",
        Message::AgentStatusWaitingForUserConfirmation => "en attente de la confirmation de l'utilisateur",
        Message::AgentStatusProcessing => "traitement",
        Message::AgentStatusPreparing => "préparation",
        Message::AgentStatusModelUnavailable => "modèle temporairement indisponible",
        Message::AgentDoneLabel => "fait",
        Message::AgentErrorLabel => "erreur",
        Message::AgentToolLabel => "outil",
        Message::AgentAgentLabel => "agent",
        Message::AgentSessionLabel => "séance",
        Message::AgentStatusLabel => "statut",
        Message::AgentInfoLabel => "infos",
        Message::AgentReadyCurrentModePrefix => "prêt, le mode actuel est",
        Message::AgentReadyCurrentModeSuffix => "",
        Message::AgentStatusDetailsSeparator => ",",
        Message::AgentPreparingToExecutePrefix => "se préparer à exécuter",
        Message::AgentActionCountSuffixSingle => "action",
        Message::AgentActionCountSuffixPlural => "actes",
        Message::AgentTurnPrefix => "tourner",
        Message::AgentTurnSuffix => "",
        Message::AgentInspectingProject => "projet d'inspection",
        Message::AgentEditingFiles => "édition de fichiers",
        Message::AgentRunningCommand => "commande en cours d'exécution",
        Message::AgentAccessingNetwork => "accéder au réseau",
        Message::AgentSearchingTools => "outils de recherche",
        Message::AgentStartingSubtask => "démarrer la sous-tâche",
        Message::AgentFileModified => "fichier modifié",
        Message::AgentCommandCompleted => "commande terminée",
        Message::AgentNetworkRequestCompleted => "demande de réseau terminée",
        Message::AgentSubtaskCompleted => "sous-tâche terminée",
        Message::AgentFailedToModifyFile => "échec de la modification du fichier",
        Message::AgentCommandFailed => "la commande a échoué",
        Message::AgentNetworkRequestFailed => "la requête réseau a échoué",
        Message::AgentSubtaskFailed => "échec de la sous-tâche",
        Message::AgentOperationFailed => "l'opération a échoué",
        Message::InteractiveInvalidLanguageValue => "langue non prise en charge",
        Message::InvalidPortValue => "--port doit être un nombre compris entre 1 et 65535",
        Message::UnknownAgentModePrefix => "inconnu --agent-mode",
        Message::NetworkPolicyMustBeOneOf => "--network-policy doit être l'un des suivants : autoriser, demander, refuser",
        Message::ForceToolSetMustBeOneOf => "--force-tool-set doit être l'un des suivants : auto, all, write",
        Message::FailedReadKeyEventPrefix => "échec de lecture de l'événement clé :",
        Message::FailedFlushPromptPrefix => "échec du vidage de l'invite :",
        Message::InteractiveRawModeFailedPrefix => "échec de l'activation du mode brut :",
    }
}

fn ja(message: Message) -> &'static str {
    match message {
        Message::ErrorPrefix => "エラー:",
        Message::NoSavedSessions => "保存されたセッションはありません。",
        Message::MessagesUnit => "件のメッセージ",
        Message::SessionLabel => "セッション:",
        Message::CreatedLabel => "作成:",
        Message::UpdatedLabel => "更新:",
        Message::MessagesLabel => "メッセージ数:",
        Message::DeletedSessionPrefix => "削除したセッション",
        Message::FailedReadPromptPrefix => "プロンプトの読み取りに失敗しました:",
        Message::PromptRequired => "メッセージを入力してください",
        Message::SessionRequiresAction => "session には操作 show または delete が必要です",
        Message::RequiresSessionIdSuffix => "にはセッション ID が必要です",
        Message::UnknownSessionActionPrefix => "不明な session 操作:",
        Message::UnknownCommandPrefix => "不明なコマンド:",
        Message::MaxIterationsPositive => "--max-iterations は 0 以上の整数である必要があります",
        Message::UnknownOptionPrefix => "不明なオプション:",
        Message::UnexpectedSessionArgs => "session コマンドに予期しない追加引数があります",
        Message::MissingEndpoint => "--endpoint または Nola 設定ファイルの llm.endpoint がありません",
        Message::ContinueOrSession => "--continue と --session は同時に指定できません",
        Message::CurrentDirFailedPrefix => "current_dir に失敗しました:",
        Message::RequiresValueSuffix => "には値が必要です",
        Message::FailedResolvePathPrefix => "パスの解決に失敗しました",
        Message::ActiveSessionLabel => "セッション:",
        Message::FailedCreateSessionDirPrefix => "セッションディレクトリの作成に失敗しました:",
        Message::FailedReadSessionPrefix => "セッションの読み取りに失敗しました",
        Message::FailedParseSessionPrefix => "セッションの解析に失敗しました",
        Message::FailedSerializeSessionPrefix => "セッションのシリアライズに失敗しました:",
        Message::FailedSaveSessionPrefix => "セッションの保存に失敗しました:",
        Message::FailedReadSessionsPrefix => "セッション一覧の読み取りに失敗しました:",
        Message::FailedReadSessionEntryPrefix => "セッション項目の読み取りに失敗しました:",
        Message::FailedDeleteSessionPrefix => "セッションの削除に失敗しました",
        Message::HomeNotSet => "HOME が設定されていないため、セッションを保存できません",
        Message::FailedReadLatestSessionPrefix => "最新セッションの読み取りに失敗しました:",
        Message::FailedSaveLatestSessionPrefix => "最新セッションの保存に失敗しました:",
        Message::InvalidSessionId => "セッション ID には英数字、'-'、'_' のみ使用できます",
        Message::FailedCreateHttpClientPrefix => "HTTP クライアントの作成に失敗しました:",
        Message::LlmRequestFailedPrefix => "LLM リクエストに失敗しました",
        Message::LlmStreamReadFailedPrefix => "LLM ストリームの読み取りに失敗しました:",
        Message::ToolRequiresApprovalPrefix => "ツール",
        Message::ToolRequiresApprovalSuffix => "には確認が必要です。",
        Message::ArgsLabel => "引数:",
        Message::AllowOncePrompt => "今回だけ許可しますか？[y/N] ",
        Message::PermissionDeniedByUser => "ユーザーが許可を拒否しました",
        Message::PermissionDeniedGuidance => {
            "権限が拒否され、操作は実行されませんでした。/allow で権限を確認してください。明示的な deny ルールは設定で変更し、再試行を依頼してください。システム権限は引き続き適用されます。"
        }
        Message::SubAgentLabel => "サブエージェント:",
        Message::TaskLabel => "タスク:",
        Message::CapabilityLabel => "権限:",
        Message::PromptLabel => "プロンプト:",
        Message::DelegateAgentCapability => {
            "設定済みの権限内でコマンドを実行、またはファイルを変更できます"
        }
        Message::DelegateAgentConfirmationRequired => {
            "サブエージェントの明示的な承認が必要です。各ツールには個別のリソース権限が引き続き適用されます。"
        }
        Message::NetworkAskNonInteractive => {
            "非対話モードではネットワークリクエスト (policy=ask) をブロックしました。続行するには --network-policy=allow を使用してください。"
        }
        Message::NetworkAskPrompt => "このモデルのネットワークリクエストを許可しますか？[y/N]: ",
        Message::NetworkAllowedByUser => "ユーザーがモデルのネットワークリクエストを許可しました。",
        Message::NetworkDeniedByUser => "ユーザーがモデルのネットワークリクエストを拒否しました。",
        Message::DisplayReadPrefix => "読み取り済み",
        Message::DisplayWrotePrefix => "書き込み済み",
        Message::DisplayEditedPrefix => "編集済み",
        Message::DisplayListedPrefix => "一覧表示済み",
        Message::EntriesUnit => "件",
        Message::MatchesUnit => "件の一致",
        Message::ReachedMaxIterationsPrefix => "\n\n最大反復回数に達しました (",
        Message::ReachedMaxIterationsSuffix => ")。現在の結果を返します。",
        Message::InteractiveQueuedRequest => "キューに入れられたリクエスト:",
        Message::InteractiveRunning => "走っています...",
        Message::InteractiveReadyForNextRequest => "次のリクエストの準備ができています",
        Message::InteractiveModelLabel => "モデル",
        Message::InteractiveParamsLabel => "パラメータ",
        Message::InteractiveWorkingDirLabel => "作業ディレクトリ",
        Message::InteractiveWorkspaceLabel => "ワークスペース",
        Message::InteractiveProviderLabel => "プロバイダー",
        Message::InteractiveProviderLocal => "ローカル構成",
        Message::InteractiveModeFieldLabel => "モード",
        Message::InteractiveNetworkFieldLabel => "ネットワーク",
        Message::InteractiveEndpointLabel => "エンドポイント",
        Message::InteractiveAvailableModels => "利用可能なモデル:",
        Message::InteractiveModelUsageHint => "使用法: /model <名前>",
        Message::InteractiveAvailableCommands => "利用可能なコマンド:",
        Message::InteractiveSlashHelpHint => "ヘルプを表示する",
        Message::InteractiveSlashModelHint => "切り替えるモデル、/model <name> をリストします。",
        Message::InteractiveSlashExecutorHint => "コア実行ユニットの一覧表示または切り替え",
        Message::InteractiveAvailableExecutors => "実行ユニット:",
        Message::InteractiveExecutorUsage => "使用法: /executor <nola|codex|claude_code|opencode>",
        Message::InteractiveExecutorSelected => "実行ユニットを選択しました:",
        Message::InteractiveExecutorUnavailable => "実行ユニットは利用できません:",
        Message::InteractiveExecutorLabel => "実行ユニット",
        Message::InteractiveSlashDebugHint => "デバッグの詳細を表示または切り替えます",
        Message::InteractiveSlashDebugUsage => "使用法: /debug 上 | /debug オフ",
        Message::InteractiveSlashAllowHint => "許可モードを設定する",
        Message::InteractiveSlashAllowUsage => "使用法: /allow — 権限選択パネルを開く",
        Message::InteractiveAllowPanelTitle => "許可モード",
        Message::InteractiveAllowPanelCautious => "慎重",
        Message::InteractiveAllowPanelCautiousDesc => "削除、上書き、機密データへのアクセス、外部送信、新規接続を確認します。通常の読み取りと新規作成は続行します。",
        Message::InteractiveAllowPanelAllowAll => "すべて許可",
        Message::InteractiveAllowPanelAllowAllDesc => "この会話では Nola の確認を省略します。システム権限は引き続き必要です。",
        Message::InteractiveAllowPanelSelected => "選択済み:",
        Message::InteractiveAllowPanelCancelled => "キャンセルされました",
        Message::InteractiveSlashGoalHint => "目標に向かって自律的に取り組む",
        Message::InteractiveSlashGoalUsage => "使用法: /goal <目標> [-c/--constraints <制約>] | /goal ステータス | /goal 再開 | /goal オフ",
        Message::InteractiveSlashInitHint => "NOLA.md プロジェクト メモリを生成します (--dry-run、--force、--output PATH)",
        Message::InteractiveSlashCdHint => "ワークスペースを切り替えて、新しいプロジェクト セッションを開始します (/cd - 戻る)",
        Message::InteractiveSlashCompactHint => "完全な再生履歴を保持しながら、モデルのコンテキストをコンパクトにします",
        Message::InteractiveSlashNewHint => "現在のワークスペースでクリーンなセッションを開始します",
        Message::InteractiveSlashPlanHint => "現在のセッションの実行計画を表示する",
        Message::InteractiveSlashJobsHint => "永続的なバックグラウンド タスクを一覧表示、検査、または停止する",
        Message::InteractiveSlashResumeHint => "セッション復元ページを開く",
        Message::InteractiveSlashContinueHint => "最新のプロジェクト セッションを続行する",
        Message::InteractiveSlashHistoryHint => "完全なセッション履歴を再印刷する",
        Message::InteractiveSlashStatusHint => "現在の実行時の状態を表示する",
        Message::InteractiveSlashExitHint => "現在のセッションを終了する",
        Message::InteractiveSlashQuitHint => "q/quit/exit も終了します",
        Message::InteractiveWelcomeTagline => "ローカルファーストアシスタント",
        Message::InteractiveWelcomeHint => "コマンドに /help と入力します。モデルを切り替えるには /model を使用します。",
        Message::InteractiveAutoValue => "自動",
        Message::InteractiveNetworkAllowValue => "許可する",
        Message::InteractiveNetworkAskValue => "尋ねる",
        Message::InteractiveNetworkDenyValue => "否定する",
        Message::InteractiveContextLabel => "コンテキスト",
        Message::InteractiveTokensLabel => "トークン",
        Message::InteractiveTokensUnit => "トークン",
        Message::InteractiveMessagesUnit => "メッセージ",
        Message::InteractiveAllowPanelErrorPrefix => "権限パネルのエラー:",
        Message::InteractiveGoalStatusPrefix => "目標:",
        Message::InteractiveGoalStatePrefix => "状態:",
        Message::InteractiveGoalStateRunning => "走っている",
        Message::InteractiveGoalStatePaused => "一時停止した",
        Message::InteractiveGoalStateNeedsUser => "ユーザーを待っています",
        Message::InteractiveGoalStateFailed => "失敗しました",
        Message::InteractiveGoalResumeStarted => "再開の目標:",
        Message::InteractiveGoalNoResumable => "このセッションでは再開可能な目標はありません。",
        Message::InteractiveWorkspaceStatusPrefix => "ワークスペース:",
        Message::InteractiveCdUsage => "使用法: /cd PATH · /cd - 前のワークスペースに戻ります",
        Message::InteractiveCompactTitle => "コンテキストの圧縮",
        Message::InteractiveCompactNoActiveSession => "アクティブなセッションがありません",
        Message::InteractiveCompactAlready => "すでにコンパクト",
        Message::InteractiveCompactCompleted => "圧縮された",
        Message::InteractiveCompactSummarized => "要約した",
        Message::InteractiveCompactKeptRecent => "最近のこと",
        Message::InteractiveCompactSaved => "保存されました",
        Message::InteractiveCompactHistoryPreserved => "完全なトランスクリプトは再生のために保存されます",
        Message::InteractiveCompactFailedPrefix => "コンテキストの圧縮に失敗しました:",
        Message::InteractiveNewSessionTitle => "新しいセッション",
        Message::InteractiveNewSessionPending => "クリーンなセッションは次のメッセージで開始されます",
        Message::InteractiveJobsTitle => "バックグラウンドタスク",
        Message::InteractiveJobsUsage => "使用法: /jobs · /jobs ID · /jobs ステータス ID · /jobs 停止 ID",
        Message::InteractiveUnknownCommandHint => "不明なコマンド。 /help と入力して、使用可能なコマンドを表示します",
        Message::InteractiveChoiceLabel => "選択してください",
        Message::InteractiveModelStatusPrefix => "モデル:",
        Message::InteractiveDebugStatusPrefix => "デバッグ:",
        Message::InteractiveAllowStatusPrefix => "許可する:",
        Message::InteractiveDefaultModelLabel => "(デフォルト)",
        Message::InteractiveTurnsLabel => "ターン",
        Message::InteractiveToolsLabel => "ツール",
        Message::InteractiveNetworkLabel => "ネットワーク",
        Message::InteractiveUnknownPath => "(不明)",
        Message::InteractiveStatusOn => "に",
        Message::InteractiveStatusOff => "オフ",
        Message::InteractiveInputPlaceholder => "メッセージを入力してください…",
        Message::InteractiveExecutionDraftPrefix => "↳",
        Message::InteractiveExecutionQueuedPrefix => "キューに追加されたフォローアップ指示:",
        Message::InteractiveExecutionInterruptHint => "Esc は現在の実行を中断します",
        Message::InteractiveExecutionInputHint => "Esc 中断 · Enter キューに追加",
        Message::AgentStatusThinking => "考えている",
        Message::AgentStatusRetryingModel => "モデル応答を再試行中",
        Message::AgentStatusStrictToolStrategy => "厳密なツール戦略",
        Message::AgentStatusChooseFromAvailableTools => "利用可能なツールから選択する",
        Message::AgentStatusReadyToContinue => "続行する準備ができています",
        Message::AgentStatusEnteredMode => "モードに入った",
        Message::AgentStatusReplyingDirectly => "直接返信する",
        Message::AgentStatusExecutingPlan => "計画の実行",
        Message::AgentStatusWaitingForUserConfirmation => "ユーザーの確認を待っています",
        Message::AgentStatusProcessing => "処理",
        Message::AgentStatusPreparing => "準備中",
        Message::AgentStatusModelUnavailable => "モデルは一時的に利用できなくなります",
        Message::AgentDoneLabel => "完了しました",
        Message::AgentErrorLabel => "エラー",
        Message::AgentToolLabel => "ツール",
        Message::AgentAgentLabel => "エージェント",
        Message::AgentSessionLabel => "セッション",
        Message::AgentStatusLabel => "ステータス",
        Message::AgentInfoLabel => "情報",
        Message::AgentReadyCurrentModePrefix => "準備完了、現在のモードは",
        Message::AgentReadyCurrentModeSuffix => "",
        Message::AgentStatusDetailsSeparator => "、",
        Message::AgentPreparingToExecutePrefix => "実行の準備中",
        Message::AgentActionCountSuffixSingle => "アクション",
        Message::AgentActionCountSuffixPlural => "アクション",
        Message::AgentTurnPrefix => "回す",
        Message::AgentTurnSuffix => "",
        Message::AgentInspectingProject => "プロジェクトの検査",
        Message::AgentEditingFiles => "ファイルの編集",
        Message::AgentRunningCommand => "実行中のコマンド",
        Message::AgentAccessingNetwork => "ネットワークにアクセスしています",
        Message::AgentSearchingTools => "検索ツール",
        Message::AgentStartingSubtask => "サブタスクの開始",
        Message::AgentFileModified => "ファイルが変更されました",
        Message::AgentCommandCompleted => "コマンドが完了しました",
        Message::AgentNetworkRequestCompleted => "ネットワークリクエストが完了しました",
        Message::AgentSubtaskCompleted => "サブタスクが完了しました",
        Message::AgentFailedToModifyFile => "ファイルの変更に失敗しました",
        Message::AgentCommandFailed => "コマンドが失敗しました",
        Message::AgentNetworkRequestFailed => "ネットワークリクエストが失敗しました",
        Message::AgentSubtaskFailed => "サブタスクが失敗しました",
        Message::AgentOperationFailed => "操作が失敗しました",
        Message::InteractiveInvalidLanguageValue => "サポートされていない言語",
        Message::InvalidPortValue => "--port は 1 ～ 65535 の数値でなければなりません",
        Message::UnknownAgentModePrefix => "不明 --agent-mode",
        Message::NetworkPolicyMustBeOneOf => "--network-policy は、許可、質問、拒否のいずれかである必要があります。",
        Message::ForceToolSetMustBeOneOf => "--force-tool-set は、auto、all、write のいずれかである必要があります。",
        Message::FailedReadKeyEventPrefix => "キーイベントの読み取りに失敗しました:",
        Message::FailedFlushPromptPrefix => "プロンプトをフラッシュできませんでした:",
        Message::InteractiveRawModeFailedPrefix => "raw モードを有効にできませんでした:",
    }
}

fn ru(message: Message) -> &'static str {
    match message {
        Message::ErrorPrefix => "ошибка:",
        Message::NoSavedSessions => "Нет сохраненных сеансов.",
        Message::MessagesUnit => "сообщ.",
        Message::SessionLabel => "сеанс:",
        Message::CreatedLabel => "создано:",
        Message::UpdatedLabel => "обновлено:",
        Message::MessagesLabel => "сообщений:",
        Message::DeletedSessionPrefix => "Удален сеанс",
        Message::FailedReadPromptPrefix => "не удалось прочитать запрос:",
        Message::PromptRequired => "требуется сообщение",
        Message::SessionRequiresAction => "session требует действие: show или delete",
        Message::RequiresSessionIdSuffix => "требует ID сеанса",
        Message::UnknownSessionActionPrefix => "неизвестное действие session:",
        Message::UnknownCommandPrefix => "неизвестная команда:",
        Message::MaxIterationsPositive => "--max-iterations должен быть неотрицательным целым числом",
        Message::UnknownOptionPrefix => "неизвестный параметр:",
        Message::UnexpectedSessionArgs => "лишние аргументы для команды session",
        Message::MissingEndpoint => "не указан --endpoint или llm.endpoint в файле конфигурации Nola",
        Message::ContinueOrSession => "используйте либо --continue, либо --session, но не оба",
        Message::CurrentDirFailedPrefix => "current_dir завершился ошибкой:",
        Message::RequiresValueSuffix => "требует значение",
        Message::FailedResolvePathPrefix => "не удалось разрешить путь",
        Message::ActiveSessionLabel => "сеанс:",
        Message::FailedCreateSessionDirPrefix => "не удалось создать каталог сеансов:",
        Message::FailedReadSessionPrefix => "не удалось прочитать сеанс",
        Message::FailedParseSessionPrefix => "не удалось разобрать сеанс",
        Message::FailedSerializeSessionPrefix => "не удалось сериализовать сеанс:",
        Message::FailedSaveSessionPrefix => "не удалось сохранить сеанс:",
        Message::FailedReadSessionsPrefix => "не удалось прочитать сеансы:",
        Message::FailedReadSessionEntryPrefix => "не удалось прочитать запись сеанса:",
        Message::FailedDeleteSessionPrefix => "не удалось удалить сеанс",
        Message::HomeNotSet => "HOME не задан; невозможно сохранять сеансы",
        Message::FailedReadLatestSessionPrefix => "не удалось прочитать последний сеанс:",
        Message::FailedSaveLatestSessionPrefix => "не удалось сохранить последний сеанс:",
        Message::InvalidSessionId => "ID сеанса может содержать только буквы, цифры, '-' и '_'",
        Message::FailedCreateHttpClientPrefix => "не удалось создать HTTP-клиент:",
        Message::LlmRequestFailedPrefix => "запрос LLM завершился ошибкой",
        Message::LlmStreamReadFailedPrefix => "не удалось прочитать поток LLM:",
        Message::ToolRequiresApprovalPrefix => "Инструмент",
        Message::ToolRequiresApprovalSuffix => "требует подтверждения.",
        Message::ArgsLabel => "аргументы:",
        Message::AllowOncePrompt => "Разрешить один раз? [y/N] ",
        Message::PermissionDeniedByUser => "Пользователь отклонил разрешение",
        Message::PermissionDeniedGuidance => {
            "Доступ отклонён; действие не выполнено. Проверьте разрешения через /allow. Измените явные правила deny в конфигурации и запросите повтор. Системные ограничения сохраняются."
        }
        Message::SubAgentLabel => "субагент:",
        Message::TaskLabel => "задача:",
        Message::CapabilityLabel => "возможность:",
        Message::PromptLabel => "запрос:",
        Message::DelegateAgentCapability => {
            "может выполнять команды или изменять файлы в рамках настроенных разрешений"
        }
        Message::DelegateAgentConfirmationRequired => {
            "Нужно явное разрешение субагента; каждый инструмент по-прежнему подчиняется собственным правам на ресурсы."
        }
        Message::NetworkAskNonInteractive => {
            "Сетевой запрос заблокирован (policy=ask) в неинтерактивном режиме. Используйте --network-policy=allow, чтобы продолжить."
        }
        Message::NetworkAskPrompt => "Разрешить сетевой запрос модели сейчас? [y/N]: ",
        Message::NetworkAllowedByUser => "Сетевой запрос модели разрешен пользователем.",
        Message::NetworkDeniedByUser => "Сетевой запрос модели отклонен пользователем.",
        Message::DisplayReadPrefix => "Прочитано",
        Message::DisplayWrotePrefix => "Записано",
        Message::DisplayEditedPrefix => "Изменено",
        Message::DisplayListedPrefix => "Выведено",
        Message::EntriesUnit => "записей",
        Message::MatchesUnit => "совпадений",
        Message::ReachedMaxIterationsPrefix => "\n\nДостигнут максимум итераций (",
        Message::ReachedMaxIterationsSuffix => "); возвращается текущий результат.",
        Message::InteractiveQueuedRequest => "запрос в очереди:",
        Message::InteractiveRunning => "бег...",
        Message::InteractiveReadyForNextRequest => "готов к следующему запросу",
        Message::InteractiveModelLabel => "Модель",
        Message::InteractiveParamsLabel => "Параметры",
        Message::InteractiveWorkingDirLabel => "Рабочая реж.",
        Message::InteractiveWorkspaceLabel => "Рабочая область",
        Message::InteractiveProviderLabel => "Поставщик",
        Message::InteractiveProviderLocal => "локальная конфигурация",
        Message::InteractiveModeFieldLabel => "Режим",
        Message::InteractiveNetworkFieldLabel => "Сеть",
        Message::InteractiveEndpointLabel => "Конечная точка",
        Message::InteractiveAvailableModels => "Доступные модели:",
        Message::InteractiveModelUsageHint => "Использование: /model <имя>",
        Message::InteractiveAvailableCommands => "Доступные команды:",
        Message::InteractiveSlashHelpHint => "Показать справку",
        Message::InteractiveSlashModelHint => "Список моделей, /model <имя> для переключения",
        Message::InteractiveSlashExecutorHint => "Показать или сменить основной исполнитель",
        Message::InteractiveAvailableExecutors => "Исполнители:",
        Message::InteractiveExecutorUsage => "Использование: /executor <nola|codex|claude_code|opencode>",
        Message::InteractiveExecutorSelected => "Выбран исполнитель:",
        Message::InteractiveExecutorUnavailable => "Исполнитель недоступен:",
        Message::InteractiveExecutorLabel => "Исполнитель",
        Message::InteractiveSlashDebugHint => "Показать или переключить сведения об отладке",
        Message::InteractiveSlashDebugUsage => "Использование: /debug на | /debug выключено",
        Message::InteractiveSlashAllowHint => "Установить режим разрешений",
        Message::InteractiveSlashAllowUsage => "Использование: /allow — открыть панель выбора разрешений.",
        Message::InteractiveAllowPanelTitle => "Режим разрешений",
        Message::InteractiveAllowPanelCautious => "Осторожный",
        Message::InteractiveAllowPanelCautiousDesc => "Подтверждать удаление, перезапись, доступ к конфиденциальным данным, отправку и новые подключения. Обычное чтение и создание файлов разрешены.",
        Message::InteractiveAllowPanelAllowAll => "Разрешить всё",
        Message::InteractiveAllowPanelAllowAllDesc => "Без подтверждений Nola в этой беседе. Системные разрешения остаются в силе.",
        Message::InteractiveAllowPanelSelected => "Выбрано:",
        Message::InteractiveAllowPanelCancelled => "Отменено",
        Message::InteractiveSlashGoalHint => "Самостоятельно работать над достижением цели",
        Message::InteractiveSlashGoalUsage => "Использование: /goal <цель> [-c/--constraints <ограничения>] | /goal статус | /goal резюме | /goal выключено",
        Message::InteractiveSlashInitHint => "Создать память проекта NOLA.md (--dry-run, --force, --output PATH)",
        Message::InteractiveSlashCdHint => "Переключите рабочую область и начните новый сеанс проекта (/cd — для возврата)",
        Message::InteractiveSlashCompactHint => "Компактный контекст модели с сохранением полной истории повторов.",
        Message::InteractiveSlashNewHint => "Запустить чистый сеанс в текущей рабочей области",
        Message::InteractiveSlashPlanHint => "Показать текущий план выполнения сеанса",
        Message::InteractiveSlashJobsHint => "Получение списка, проверка или остановка сохраненных фоновых задач",
        Message::InteractiveSlashResumeHint => "Откройте страницу восстановления сеанса",
        Message::InteractiveSlashContinueHint => "Продолжить последнюю сессию проекта",
        Message::InteractiveSlashHistoryHint => "Перепечатать полную историю сеансов",
        Message::InteractiveSlashStatusHint => "Показать текущее состояние выполнения",
        Message::InteractiveSlashExitHint => "Выйти из текущего сеанса",
        Message::InteractiveSlashQuitHint => "q/quit/exit также выходит",
        Message::InteractiveWelcomeTagline => "местный первый помощник",
        Message::InteractiveWelcomeHint => "Введите /help для команд · используйте /model для переключения моделей.",
        Message::InteractiveAutoValue => "авто",
        Message::InteractiveNetworkAllowValue => "разрешить",
        Message::InteractiveNetworkAskValue => "спросить",
        Message::InteractiveNetworkDenyValue => "отрицать",
        Message::InteractiveContextLabel => "Контекст",
        Message::InteractiveTokensLabel => "жетоны",
        Message::InteractiveTokensUnit => "жетоны",
        Message::InteractiveMessagesUnit => "сообщения",
        Message::InteractiveAllowPanelErrorPrefix => "ошибка панели разрешений:",
        Message::InteractiveGoalStatusPrefix => "цель:",
        Message::InteractiveGoalStatePrefix => "состояние:",
        Message::InteractiveGoalStateRunning => "бег",
        Message::InteractiveGoalStatePaused => "остановился",
        Message::InteractiveGoalStateNeedsUser => "ожидание пользователя",
        Message::InteractiveGoalStateFailed => "не удалось",
        Message::InteractiveGoalResumeStarted => "возобновляемая цель:",
        Message::InteractiveGoalNoResumable => "В этом сеансе нет возобновляемой цели.",
        Message::InteractiveWorkspaceStatusPrefix => "рабочая область:",
        Message::InteractiveCdUsage => "Использование: /cd PATH · /cd — возврат к предыдущей рабочей области.",
        Message::InteractiveCompactTitle => "сжатие контекста",
        Message::InteractiveCompactNoActiveSession => "нет активной сессии",
        Message::InteractiveCompactAlready => "уже компактный",
        Message::InteractiveCompactCompleted => "уплотненный",
        Message::InteractiveCompactSummarized => "обобщенный",
        Message::InteractiveCompactKeptRecent => "сохраняется последним",
        Message::InteractiveCompactSaved => "сохранено",
        Message::InteractiveCompactHistoryPreserved => "полная стенограмма сохраняется для повтора",
        Message::InteractiveCompactFailedPrefix => "сжатие контекста не удалось:",
        Message::InteractiveNewSessionTitle => "новая сессия",
        Message::InteractiveNewSessionPending => "чистый сеанс начнется со следующего сообщения",
        Message::InteractiveJobsTitle => "фоновые задачи",
        Message::InteractiveJobsUsage => "Использование: /jobs · /jobs ID · /jobs статус ID · /jobs стоп ID",
        Message::InteractiveUnknownCommandHint => "Неизвестная команда; введите /help, чтобы увидеть доступные команды",
        Message::InteractiveChoiceLabel => "Пожалуйста, выберите",
        Message::InteractiveModelStatusPrefix => "модель:",
        Message::InteractiveDebugStatusPrefix => "отладка:",
        Message::InteractiveAllowStatusPrefix => "разрешить:",
        Message::InteractiveDefaultModelLabel => "(по умолчанию)",
        Message::InteractiveTurnsLabel => "поворачивается",
        Message::InteractiveToolsLabel => "инструменты",
        Message::InteractiveNetworkLabel => "сеть",
        Message::InteractiveUnknownPath => "(неизвестно)",
        Message::InteractiveStatusOn => "на",
        Message::InteractiveStatusOff => "выкл.",
        Message::InteractiveInputPlaceholder => "Введите сообщение…",
        Message::InteractiveExecutionDraftPrefix => "↳",
        Message::InteractiveExecutionQueuedPrefix => "Последующие инструкции в очереди:",
        Message::InteractiveExecutionInterruptHint => "Esc прерывает текущий запуск",
        Message::InteractiveExecutionInputHint => "Esc прервать · Enter в очередь",
        Message::AgentStatusThinking => "мышление",
        Message::AgentStatusRetryingModel => "повторная попытка ответа модели",
        Message::AgentStatusStrictToolStrategy => "строгая инструментальная стратегия",
        Message::AgentStatusChooseFromAvailableTools => "выбрать из доступных инструментов",
        Message::AgentStatusReadyToContinue => "готов продолжить",
        Message::AgentStatusEnteredMode => "вошёл в режим",
        Message::AgentStatusReplyingDirectly => "отвечая напрямую",
        Message::AgentStatusExecutingPlan => "план выполнения",
        Message::AgentStatusWaitingForUserConfirmation => "ожидание подтверждения пользователя",
        Message::AgentStatusProcessing => "обработка",
        Message::AgentStatusPreparing => "подготовка",
        Message::AgentStatusModelUnavailable => "модель временно недоступна",
        Message::AgentDoneLabel => "сделано",
        Message::AgentErrorLabel => "ошибка",
        Message::AgentToolLabel => "инструмент",
        Message::AgentAgentLabel => "агент",
        Message::AgentSessionLabel => "сессия",
        Message::AgentStatusLabel => "статус",
        Message::AgentInfoLabel => "информация",
        Message::AgentReadyCurrentModePrefix => "готов, текущий режим",
        Message::AgentReadyCurrentModeSuffix => "",
        Message::AgentStatusDetailsSeparator => ",",
        Message::AgentPreparingToExecutePrefix => "готовится к исполнению",
        Message::AgentActionCountSuffixSingle => "действие",
        Message::AgentActionCountSuffixPlural => "действия",
        Message::AgentTurnPrefix => "повернуть",
        Message::AgentTurnSuffix => "",
        Message::AgentInspectingProject => "проверка проекта",
        Message::AgentEditingFiles => "редактирование файлов",
        Message::AgentRunningCommand => "работающая команда",
        Message::AgentAccessingNetwork => "доступ к сети",
        Message::AgentSearchingTools => "инструменты поиска",
        Message::AgentStartingSubtask => "запуск подзадачи",
        Message::AgentFileModified => "файл изменен",
        Message::AgentCommandCompleted => "команда выполнена",
        Message::AgentNetworkRequestCompleted => "сетевой запрос выполнен",
        Message::AgentSubtaskCompleted => "подзадача завершена",
        Message::AgentFailedToModifyFile => "не удалось изменить файл",
        Message::AgentCommandFailed => "команда не удалась",
        Message::AgentNetworkRequestFailed => "сетевой запрос не выполнен",
        Message::AgentSubtaskFailed => "подзадача не удалась",
        Message::AgentOperationFailed => "операция не удалась",
        Message::InteractiveInvalidLanguageValue => "неподдерживаемый язык",
        Message::InvalidPortValue => "--port должно быть числом от 1 до 65535.",
        Message::UnknownAgentModePrefix => "неизвестно --agent-mode",
        Message::NetworkPolicyMustBeOneOf => "--network-policy должно быть одним из: разрешить, спросить, запретить",
        Message::ForceToolSetMustBeOneOf => "--force-tool-set должно быть одним из: авто, все, запись",
        Message::FailedReadKeyEventPrefix => "не удалось прочитать ключевое событие:",
        Message::FailedFlushPromptPrefix => "не удалось сбросить подсказку:",
        Message::InteractiveRawModeFailedPrefix => "не удалось включить необработанный режим:",
    }
}

fn normalized_locale_tag(tag: &str) -> String {
    tag.split(':')
        .next()
        .unwrap_or(tag)
        .split('.')
        .next()
        .unwrap_or(tag)
        .replace('_', "-")
        .to_ascii_lowercase()
}

fn is_explicit_english(tag: &str) -> bool {
    let normalized = normalized_locale_tag(tag);
    normalized == "en" || normalized.starts_with("en-")
}

fn parse_tool_display_error(text: &str) -> Option<ToolDisplayError<'_>> {
    if let Some(name) = quoted_suffix(text, "Unknown tool '") {
        return Some(ToolDisplayError::UnknownTool(name));
    }
    if let Some(rest) = text.strip_prefix("Not a file: ") {
        return Some(ToolDisplayError::NotFile(rest));
    }
    if let Some(rest) = text.strip_prefix("Read failed: ") {
        return Some(ToolDisplayError::ReadFailed(rest));
    }
    if text == "content is required" {
        return Some(ToolDisplayError::ContentRequired);
    }
    if let Some(rest) = text
        .strip_prefix("content exceeds ")
        .and_then(|value| value.strip_suffix(" bytes"))
    {
        return Some(ToolDisplayError::ContentExceeds(rest));
    }
    if let Some(rest) = text.strip_prefix("Create parent directory failed: ") {
        return Some(ToolDisplayError::CreateParentFailed(rest));
    }
    if let Some(rest) = text.strip_prefix("Write failed: ") {
        return Some(ToolDisplayError::WriteFailed(rest));
    }
    if text == "old_text is required" {
        return Some(ToolDisplayError::OldTextRequired);
    }
    if text == "new_text is required" {
        return Some(ToolDisplayError::NewTextRequired);
    }
    if text == "old_text was not found" {
        return Some(ToolDisplayError::OldTextNotFound);
    }
    if let Some(rest) = text
        .strip_prefix("old_text matched ")
        .and_then(|value| value.strip_suffix("; set replace_all=true or provide more context"))
    {
        return Some(ToolDisplayError::OldTextMatched(rest));
    }
    if let Some(rest) = text.strip_prefix("Not a directory: ") {
        return Some(ToolDisplayError::NotDirectory(rest));
    }
    if let Some(rest) = text.strip_prefix("List failed: ") {
        return Some(ToolDisplayError::ListFailed(rest));
    }
    if let Some(rest) = text.strip_prefix("invalid regex: ") {
        return Some(ToolDisplayError::InvalidRegex(rest));
    }
    if text == "command too long" {
        return Some(ToolDisplayError::CommandTooLong);
    }
    if text == "shell metacharacters/substitutions are not allowed" {
        return Some(ToolDisplayError::ShellMetacharacters);
    }
    if text == "No bash prefixes whitelisted. Use --bash-prefix." {
        return Some(ToolDisplayError::NoBashPrefixes);
    }
    if let Some(head) = text
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix("' is not in the bash allow-list"))
    {
        return Some(ToolDisplayError::NotInBashAllowList(head));
    }
    if let Some(rest) = text.strip_prefix("cwd is not a directory: ") {
        return Some(ToolDisplayError::CwdNotDirectory(rest));
    }
    if let Some(rest) = text.strip_prefix("temp stdout failed: ") {
        return Some(ToolDisplayError::TempStdoutFailed(rest));
    }
    if let Some(rest) = text.strip_prefix("temp stderr failed: ") {
        return Some(ToolDisplayError::TempStderrFailed(rest));
    }
    if let Some(rest) = text.strip_prefix("Exec failed: ") {
        return Some(ToolDisplayError::ExecFailed(rest));
    }
    if let Some(rest) = text
        .strip_prefix("Command timed out after ")
        .and_then(|value| value.strip_suffix('s'))
    {
        return Some(ToolDisplayError::CommandTimedOut(rest));
    }
    if let Some(rest) = text.strip_prefix("Wait failed: ") {
        return Some(ToolDisplayError::WaitFailed(rest));
    }
    if let Some(rest) = text.strip_suffix(" is required") {
        return Some(ToolDisplayError::Required(rest));
    }
    if text == "no allowed directories configured" {
        return Some(ToolDisplayError::NoAllowedDirs);
    }
    if let Some(rest) = text.strip_prefix("failed to resolve ") {
        return Some(ToolDisplayError::FailedResolve(rest));
    }
    if let Some(rest) = text.strip_prefix("path does not exist: ") {
        return Some(ToolDisplayError::PathDoesNotExist(rest));
    }
    if let Some(rest) = text.strip_prefix("failed to resolve parent for ") {
        return Some(ToolDisplayError::FailedResolveParent(rest));
    }
    if let Some(rest) = text.strip_prefix("path is outside allowed directories: ") {
        return Some(ToolDisplayError::OutsideAllowed(rest));
    }
    if let Some(rest) = text.strip_prefix("invalid glob pattern: ") {
        return Some(ToolDisplayError::InvalidGlob(rest));
    }
    None
}

fn format_tool_display_error(language: Language, error: ToolDisplayError<'_>) -> String {
    match language {
        Language::En => match error {
            ToolDisplayError::UnknownTool(name) => format!("Unknown tool '{name}'"),
            ToolDisplayError::NotFile(path) => format!("Not a file: {path}"),
            ToolDisplayError::ReadFailed(err) => format!("Read failed: {err}"),
            ToolDisplayError::ContentRequired => "content is required".to_string(),
            ToolDisplayError::ContentExceeds(size) => format!("content exceeds {size} bytes"),
            ToolDisplayError::CreateParentFailed(err) => {
                format!("Create parent directory failed: {err}")
            }
            ToolDisplayError::WriteFailed(err) => format!("Write failed: {err}"),
            ToolDisplayError::OldTextRequired => "old_text is required".to_string(),
            ToolDisplayError::NewTextRequired => "new_text is required".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text was not found".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text matched {count} times; set replace_all=true or provide more context"
            ),
            ToolDisplayError::NotDirectory(path) => format!("Not a directory: {path}"),
            ToolDisplayError::ListFailed(err) => format!("List failed: {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("invalid regex: {err}"),
            ToolDisplayError::CommandTooLong => "command too long".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "shell metacharacters/substitutions are not allowed".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "No bash prefixes whitelisted. Use --bash-prefix.".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' is not in the bash allow-list")
            }
            ToolDisplayError::CwdNotDirectory(path) => {
                format!("cwd is not a directory: {path}")
            }
            ToolDisplayError::TempStdoutFailed(err) => format!("temp stdout failed: {err}"),
            ToolDisplayError::TempStderrFailed(err) => format!("temp stderr failed: {err}"),
            ToolDisplayError::ExecFailed(err) => format!("Exec failed: {err}"),
            ToolDisplayError::CommandTimedOut(sec) => {
                format!("Command timed out after {sec}s")
            }
            ToolDisplayError::WaitFailed(err) => format!("Wait failed: {err}"),
            ToolDisplayError::Required(key) => format!("{key} is required"),
            ToolDisplayError::NoAllowedDirs => "no allowed directories configured".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("failed to resolve {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("path does not exist: {path}"),
            ToolDisplayError::FailedResolveParent(path) => {
                format!("failed to resolve parent for {path}")
            }
            ToolDisplayError::OutsideAllowed(path) => {
                format!("path is outside allowed directories: {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("invalid glob pattern: {err}"),
        },
        Language::ZhHans => match error {
            ToolDisplayError::UnknownTool(name) => format!("未知工具 '{name}'"),
            ToolDisplayError::NotFile(path) => format!("不是文件：{path}"),
            ToolDisplayError::ReadFailed(err) => format!("读取失败：{err}"),
            ToolDisplayError::ContentRequired => "缺少 content".to_string(),
            ToolDisplayError::ContentExceeds(size) => format!("content 超过 {size} 字节"),
            ToolDisplayError::CreateParentFailed(err) => format!("创建父目录失败：{err}"),
            ToolDisplayError::WriteFailed(err) => format!("写入失败：{err}"),
            ToolDisplayError::OldTextRequired => "缺少 old_text".to_string(),
            ToolDisplayError::NewTextRequired => "缺少 new_text".to_string(),
            ToolDisplayError::OldTextNotFound => "未找到 old_text".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text 匹配 {count} 次；请设置 replace_all=true 或提供更多上下文"
            ),
            ToolDisplayError::NotDirectory(path) => format!("不是目录：{path}"),
            ToolDisplayError::ListFailed(err) => format!("列出目录失败：{err}"),
            ToolDisplayError::InvalidRegex(err) => format!("正则表达式无效：{err}"),
            ToolDisplayError::CommandTooLong => "命令过长".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "不允许使用 shell 元字符或替换表达式".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "未配置允许的 Bash 前缀。请使用 --bash-prefix。".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' 不在 Bash 允许列表中")
            }
            ToolDisplayError::CwdNotDirectory(path) => format!("cwd 不是目录：{path}"),
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("创建临时 stdout 文件失败：{err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("创建临时 stderr 文件失败：{err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("执行失败：{err}"),
            ToolDisplayError::CommandTimedOut(sec) => format!("命令在 {sec}s 后超时"),
            ToolDisplayError::WaitFailed(err) => format!("等待命令结束失败：{err}"),
            ToolDisplayError::Required(key) => format!("缺少 {key}"),
            ToolDisplayError::NoAllowedDirs => "未配置允许访问的目录".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("解析路径失败 {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("路径不存在：{path}"),
            ToolDisplayError::FailedResolveParent(path) => format!("解析父目录失败：{path}"),
            ToolDisplayError::OutsideAllowed(path) => {
                format!("路径不在允许访问的目录内：{path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("glob 模式无效：{err}"),
        },
        Language::De => match error {
            ToolDisplayError::UnknownTool(name) => format!("Unbekanntes Tool '{name}'"),
            ToolDisplayError::NotFile(path) => format!("Keine Datei: {path}"),
            ToolDisplayError::ReadFailed(err) => format!("Lesen fehlgeschlagen: {err}"),
            ToolDisplayError::ContentRequired => "content ist erforderlich".to_string(),
            ToolDisplayError::ContentExceeds(size) => {
                format!("content überschreitet {size} Bytes")
            }
            ToolDisplayError::CreateParentFailed(err) => {
                format!("Übergeordnetes Verzeichnis konnte nicht erstellt werden: {err}")
            }
            ToolDisplayError::WriteFailed(err) => format!("Schreiben fehlgeschlagen: {err}"),
            ToolDisplayError::OldTextRequired => "old_text ist erforderlich".to_string(),
            ToolDisplayError::NewTextRequired => "new_text ist erforderlich".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text wurde nicht gefunden".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text wurde {count} Mal gefunden; setze replace_all=true oder gib mehr Kontext an"
            ),
            ToolDisplayError::NotDirectory(path) => format!("Kein Verzeichnis: {path}"),
            ToolDisplayError::ListFailed(err) => format!("Auflisten fehlgeschlagen: {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("ungültiger Regex: {err}"),
            ToolDisplayError::CommandTooLong => "Befehl ist zu lang".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "Shell-Metazeichen/Substitutionen sind nicht erlaubt".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "Keine Bash-Präfixe erlaubt. Nutze --bash-prefix.".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' ist nicht in der Bash-Erlaubnisliste")
            }
            ToolDisplayError::CwdNotDirectory(path) => {
                format!("cwd ist kein Verzeichnis: {path}")
            }
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("temporäre stdout-Datei fehlgeschlagen: {err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("temporäre stderr-Datei fehlgeschlagen: {err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("Ausführung fehlgeschlagen: {err}"),
            ToolDisplayError::CommandTimedOut(sec) => {
                format!("Befehl nach {sec}s abgebrochen")
            }
            ToolDisplayError::WaitFailed(err) => format!("Warten fehlgeschlagen: {err}"),
            ToolDisplayError::Required(key) => format!("{key} ist erforderlich"),
            ToolDisplayError::NoAllowedDirs => "keine erlaubten Verzeichnisse konfiguriert".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("Auflösung fehlgeschlagen {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("Pfad existiert nicht: {path}"),
            ToolDisplayError::FailedResolveParent(path) => {
                format!("übergeordnetes Verzeichnis konnte nicht aufgelöst werden: {path}")
            }
            ToolDisplayError::OutsideAllowed(path) => {
                format!("Pfad liegt außerhalb der erlaubten Verzeichnisse: {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("ungültiges Glob-Muster: {err}"),
        },
        Language::Es => match error {
            ToolDisplayError::UnknownTool(name) => format!("Herramienta desconocida '{name}'"),
            ToolDisplayError::NotFile(path) => format!("No es un archivo: {path}"),
            ToolDisplayError::ReadFailed(err) => format!("Falló la lectura: {err}"),
            ToolDisplayError::ContentRequired => "content es obligatorio".to_string(),
            ToolDisplayError::ContentExceeds(size) => {
                format!("content supera {size} bytes")
            }
            ToolDisplayError::CreateParentFailed(err) => {
                format!("No se pudo crear el directorio padre: {err}")
            }
            ToolDisplayError::WriteFailed(err) => format!("Falló la escritura: {err}"),
            ToolDisplayError::OldTextRequired => "old_text es obligatorio".to_string(),
            ToolDisplayError::NewTextRequired => "new_text es obligatorio".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text no se encontró".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text coincidió {count} veces; usa replace_all=true o da más contexto"
            ),
            ToolDisplayError::NotDirectory(path) => format!("No es un directorio: {path}"),
            ToolDisplayError::ListFailed(err) => format!("Falló el listado: {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("regex no válida: {err}"),
            ToolDisplayError::CommandTooLong => "comando demasiado largo".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "no se permiten metacaracteres ni sustituciones de shell".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "No hay prefijos Bash permitidos. Usa --bash-prefix.".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' no está en la lista permitida de Bash")
            }
            ToolDisplayError::CwdNotDirectory(path) => {
                format!("cwd no es un directorio: {path}")
            }
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("falló el stdout temporal: {err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("falló el stderr temporal: {err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("Falló la ejecución: {err}"),
            ToolDisplayError::CommandTimedOut(sec) => {
                format!("El comando agotó el tiempo tras {sec}s")
            }
            ToolDisplayError::WaitFailed(err) => format!("Falló la espera: {err}"),
            ToolDisplayError::Required(key) => format!("{key} es obligatorio"),
            ToolDisplayError::NoAllowedDirs => "no hay directorios permitidos configurados".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("no se pudo resolver {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("la ruta no existe: {path}"),
            ToolDisplayError::FailedResolveParent(path) => {
                format!("no se pudo resolver el directorio padre de {path}")
            }
            ToolDisplayError::OutsideAllowed(path) => {
                format!("la ruta está fuera de los directorios permitidos: {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("patrón glob no válido: {err}"),
        },
        Language::Fr => match error {
            ToolDisplayError::UnknownTool(name) => format!("Outil inconnu '{name}'"),
            ToolDisplayError::NotFile(path) => format!("Ce n'est pas un fichier : {path}"),
            ToolDisplayError::ReadFailed(err) => format!("Échec de lecture : {err}"),
            ToolDisplayError::ContentRequired => "content est requis".to_string(),
            ToolDisplayError::ContentExceeds(size) => {
                format!("content dépasse {size} octets")
            }
            ToolDisplayError::CreateParentFailed(err) => {
                format!("Échec de création du répertoire parent : {err}")
            }
            ToolDisplayError::WriteFailed(err) => format!("Échec d'écriture : {err}"),
            ToolDisplayError::OldTextRequired => "old_text est requis".to_string(),
            ToolDisplayError::NewTextRequired => "new_text est requis".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text est introuvable".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text correspond {count} fois ; définissez replace_all=true ou ajoutez du contexte"
            ),
            ToolDisplayError::NotDirectory(path) => format!("Ce n'est pas un répertoire : {path}"),
            ToolDisplayError::ListFailed(err) => format!("Échec du listage : {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("regex invalide : {err}"),
            ToolDisplayError::CommandTooLong => "commande trop longue".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "les métacaractères/substitutions shell ne sont pas autorisés".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "Aucun préfixe Bash autorisé. Utilisez --bash-prefix.".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' n'est pas dans la liste Bash autorisée")
            }
            ToolDisplayError::CwdNotDirectory(path) => {
                format!("cwd n'est pas un répertoire : {path}")
            }
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("échec du stdout temporaire : {err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("échec du stderr temporaire : {err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("Échec d'exécution : {err}"),
            ToolDisplayError::CommandTimedOut(sec) => {
                format!("La commande a expiré après {sec}s")
            }
            ToolDisplayError::WaitFailed(err) => format!("Échec d'attente : {err}"),
            ToolDisplayError::Required(key) => format!("{key} est requis"),
            ToolDisplayError::NoAllowedDirs => "aucun répertoire autorisé configuré".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("échec de résolution {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("le chemin n'existe pas : {path}"),
            ToolDisplayError::FailedResolveParent(path) => {
                format!("échec de résolution du parent pour {path}")
            }
            ToolDisplayError::OutsideAllowed(path) => {
                format!("le chemin est hors des répertoires autorisés : {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("motif glob invalide : {err}"),
        },
        Language::Ja => match error {
            ToolDisplayError::UnknownTool(name) => format!("不明なツール '{name}'"),
            ToolDisplayError::NotFile(path) => format!("ファイルではありません: {path}"),
            ToolDisplayError::ReadFailed(err) => format!("読み取りに失敗しました: {err}"),
            ToolDisplayError::ContentRequired => "content が必要です".to_string(),
            ToolDisplayError::ContentExceeds(size) => {
                format!("content が {size} バイトを超えています")
            }
            ToolDisplayError::CreateParentFailed(err) => {
                format!("親ディレクトリの作成に失敗しました: {err}")
            }
            ToolDisplayError::WriteFailed(err) => format!("書き込みに失敗しました: {err}"),
            ToolDisplayError::OldTextRequired => "old_text が必要です".to_string(),
            ToolDisplayError::NewTextRequired => "new_text が必要です".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text が見つかりません".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text が {count} 回一致しました。replace_all=true を設定するか、さらに文脈を指定してください"
            ),
            ToolDisplayError::NotDirectory(path) => format!("ディレクトリではありません: {path}"),
            ToolDisplayError::ListFailed(err) => format!("一覧取得に失敗しました: {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("正規表現が無効です: {err}"),
            ToolDisplayError::CommandTooLong => "コマンドが長すぎます".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "shell メタ文字または置換式は使用できません".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "許可された Bash 接頭辞がありません。--bash-prefix を使用してください。".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' は Bash 許可リストにありません")
            }
            ToolDisplayError::CwdNotDirectory(path) => {
                format!("cwd はディレクトリではありません: {path}")
            }
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("一時 stdout の作成に失敗しました: {err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("一時 stderr の作成に失敗しました: {err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("実行に失敗しました: {err}"),
            ToolDisplayError::CommandTimedOut(sec) => {
                format!("コマンドが {sec}s 後にタイムアウトしました")
            }
            ToolDisplayError::WaitFailed(err) => format!("待機に失敗しました: {err}"),
            ToolDisplayError::Required(key) => format!("{key} が必要です"),
            ToolDisplayError::NoAllowedDirs => "許可されたディレクトリが設定されていません".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("パスの解決に失敗しました {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("パスが存在しません: {path}"),
            ToolDisplayError::FailedResolveParent(path) => {
                format!("親ディレクトリの解決に失敗しました: {path}")
            }
            ToolDisplayError::OutsideAllowed(path) => {
                format!("パスは許可されたディレクトリの外です: {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("glob パターンが無効です: {err}"),
        },
        Language::Ru => match error {
            ToolDisplayError::UnknownTool(name) => format!("Неизвестный инструмент '{name}'"),
            ToolDisplayError::NotFile(path) => format!("Не файл: {path}"),
            ToolDisplayError::ReadFailed(err) => format!("Ошибка чтения: {err}"),
            ToolDisplayError::ContentRequired => "требуется content".to_string(),
            ToolDisplayError::ContentExceeds(size) => {
                format!("content превышает {size} байт")
            }
            ToolDisplayError::CreateParentFailed(err) => {
                format!("не удалось создать родительский каталог: {err}")
            }
            ToolDisplayError::WriteFailed(err) => format!("Ошибка записи: {err}"),
            ToolDisplayError::OldTextRequired => "требуется old_text".to_string(),
            ToolDisplayError::NewTextRequired => "требуется new_text".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text не найден".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text найден {count} раз; задайте replace_all=true или добавьте контекст"
            ),
            ToolDisplayError::NotDirectory(path) => format!("Не каталог: {path}"),
            ToolDisplayError::ListFailed(err) => format!("Ошибка списка: {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("недопустимое regex: {err}"),
            ToolDisplayError::CommandTooLong => "команда слишком длинная".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "метасимволы/подстановки shell не разрешены".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "Нет разрешенных Bash-префиксов. Используйте --bash-prefix.".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' отсутствует в списке разрешений Bash")
            }
            ToolDisplayError::CwdNotDirectory(path) => {
                format!("cwd не является каталогом: {path}")
            }
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("ошибка временного stdout: {err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("ошибка временного stderr: {err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("Ошибка выполнения: {err}"),
            ToolDisplayError::CommandTimedOut(sec) => {
                format!("Команда превысила время ожидания {sec}s")
            }
            ToolDisplayError::WaitFailed(err) => format!("Ошибка ожидания: {err}"),
            ToolDisplayError::Required(key) => format!("требуется {key}"),
            ToolDisplayError::NoAllowedDirs => "не настроены разрешенные каталоги".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("не удалось разрешить {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("путь не существует: {path}"),
            ToolDisplayError::FailedResolveParent(path) => {
                format!("не удалось разрешить родительский каталог для {path}")
            }
            ToolDisplayError::OutsideAllowed(path) => {
                format!("путь вне разрешенных каталогов: {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("недопустимый glob-шаблон: {err}"),
        },
    }
}

fn quoted_suffix<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.strip_prefix(prefix)?.strip_suffix('\'')
}

#[cfg(test)]
mod tests {
    #[test]
    fn permission_profile_copy_does_not_promise_unrestricted_execution() {
        use super::*;
        assert!(text_for(Language::ZhHans, Message::InteractiveAllowPanelAllowAllDesc).contains("系统权限"));
        assert!(!text_for(Language::ZhHans, Message::InteractiveAllowPanelCautiousDesc).contains("仅在"));
        let allow_all = text_for(Language::En, Message::InteractiveAllowPanelAllowAllDesc);
        assert!(allow_all.contains("No Nola approval prompts"));
        assert!(allow_all.contains("System permissions still apply"));
    }
    use super::{
        browser_accept_language_for, browser_locale_for, help_text_for, interaction_text_for,
        language_from_tag, localize_tool_display_name_for, localize_tool_error_display_for,
        text_for, ui_text_for, Language, Message, UiText,
    };

    #[test]
    fn detects_supported_locale_tags() {
        assert_eq!(language_from_tag("zh-Hans"), Language::ZhHans);
        assert_eq!(language_from_tag("zh_CN.UTF-8"), Language::ZhHans);
        assert_eq!(language_from_tag("en_US.UTF-8"), Language::En);
        assert_eq!(language_from_tag("de_DE.UTF-8"), Language::De);
        assert_eq!(language_from_tag("es-ES"), Language::Es);
        assert_eq!(language_from_tag("fr_FR.UTF-8"), Language::Fr);
        assert_eq!(language_from_tag("ja_JP.UTF-8"), Language::Ja);
        assert_eq!(language_from_tag("ru_RU.UTF-8"), Language::Ru);
    }

    #[test]
    fn falls_back_to_english_for_unknown_locale_tags() {
        assert_eq!(language_from_tag("C"), Language::En);
        assert_eq!(language_from_tag("it_IT.UTF-8"), Language::En);
    }

    #[test]
    fn localizes_common_tool_display_names_without_hiding_unknown_names() {
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "mcp_resources_read"),
            "读取 MCP 资源"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::En, "mcp_resource_templates_read"),
            "Read MCP template resource"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "git_diff"),
            "检查变更"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "project_overview"),
            "查看项目概览"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "codebase_search"),
            "搜索文件"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::En, "codebase_index_status"),
            "Inspect workspace"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "focused_context"),
            "检查工作区"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "find_files"),
            "浏览目录"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "tool_search"),
            "搜索工具"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "computer_click"),
            "操作电脑"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::En, "read_file"),
            "Read file"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "custom_tool"),
            "工具 · custom_tool"
        );
    }

    #[test]
    fn localizes_interaction_panels_without_cross_language_fallbacks() {
        let english = interaction_text_for(Language::En);
        assert_eq!(english.custom_answer, "Custom answer…");
        assert_eq!(english.resume_title, "Resume session");
        assert!(!english.selection_hint.contains('选'));
        assert!(!english.question_cancelled.contains('取'));

        let chinese = interaction_text_for(Language::ZhHans);
        assert_eq!(chinese.custom_answer, "自定义回答…");
        assert_eq!(chinese.resume_title, "恢复会话");
        assert!(!chinese.selection_hint.contains("confirm"));
        assert!(!chinese.validation_pipeline_rejected.contains("must"));
    }

    #[test]
    fn localizes_secondary_cli_surfaces_for_every_supported_language() {
        let languages = [
            Language::ZhHans,
            Language::En,
            Language::De,
            Language::Es,
            Language::Fr,
            Language::Ja,
            Language::Ru,
        ];
        let keys = [
            UiText::Answer,
            UiText::SessionBusy,
            UiText::TasksUsage,
            UiText::HelpAgent,
            UiText::NoBackgroundTasks,
            UiText::WaitingPermission,
            UiText::ConfigNotFound,
            UiText::ExecutionPlan,
        ];
        for language in languages {
            for key in keys {
                assert!(
                    !ui_text_for(language, key).trim().is_empty(),
                    "{language:?} {key:?}"
                );
            }
        }
        assert_eq!(ui_text_for(Language::ZhHans, UiText::Answer), "回答");
        assert_eq!(
            ui_text_for(Language::Ja, UiText::WaitingPermission),
            "承認待ち"
        );
        assert_eq!(
            ui_text_for(Language::De, UiText::NoAvailableModels),
            "Keine Modelle verfügbar"
        );
        assert!(!ui_text_for(Language::Fr, UiText::HelpTasks).contains('权'));
    }

    #[test]
    fn provides_localized_help_text_for_supported_languages() {
        let cases = [
            (Language::ZhHans, "用法："),
            (Language::En, "Usage:"),
            (Language::De, "Verwendung:"),
            (Language::Es, "Uso:"),
            (Language::Fr, "Utilisation :"),
            (Language::Ja, "使い方:"),
            (Language::Ru, "Использование:"),
        ];
        for (language, marker) in cases {
            assert!(help_text_for(language).contains(marker));
        }
    }

    #[test]
    fn setup_yes_never_advertises_agent_tool_authority() {
        for language in [Language::ZhHans, Language::En, Language::De, Language::Es,
            Language::Fr, Language::Ja, Language::Ru] {
            let line = help_text_for(language).lines().find(|line|line.contains("--yes, -y")).unwrap();
            assert!(line.contains("init/wiki"));
            assert!(!text_for(language, Message::PermissionDeniedGuidance).contains("--yes"));
            assert!(!text_for(language, Message::DelegateAgentConfirmationRequired).contains("--yes"));
        }
        assert!(help_text_for(Language::En).contains("never grants Agent tool permission"));
    }

    #[test]
    fn primary_help_does_not_advertise_removed_agent_modes() {
        for language in [Language::ZhHans, Language::En] {
            let help = help_text_for(language);
            assert!(!help.contains("--swe-mode"));
            assert!(!help.contains("--agent-mode"));
            assert!(help.contains("--resume"));
            assert!(help.contains("NOLA_CLI_DEBUG_LOG_MAX_BYTES"));
        }
    }

    #[test]
    fn provides_browser_locale_for_supported_languages() {
        let cases = [
            (Language::ZhHans, "zh-CN", "zh-CN,zh;q=0.9"),
            (Language::En, "en-US", "en-US,en;q=0.9"),
            (Language::De, "de-DE", "de-DE,de;q=0.9"),
            (Language::Es, "es-ES", "es-ES,es;q=0.9"),
            (Language::Fr, "fr-FR", "fr-FR,fr;q=0.9"),
            (Language::Ja, "ja-JP", "ja-JP,ja;q=0.9"),
            (Language::Ru, "ru-RU", "ru-RU,ru;q=0.9"),
        ];
        for (language, locale, accept_language_prefix) in cases {
            assert_eq!(browser_locale_for(language), locale);
            assert!(
                browser_accept_language_for(language).starts_with(accept_language_prefix),
                "{language:?} should prioritize its own browser language"
            );
        }
    }

    #[test]
    fn localizes_interaction_guidance_for_supported_languages() {
        let languages = [
            Language::ZhHans,
            Language::En,
            Language::De,
            Language::Es,
            Language::Fr,
            Language::Ja,
            Language::Ru,
        ];
        let messages = [
            Message::PermissionDeniedGuidance,
            Message::SubAgentLabel,
            Message::TaskLabel,
            Message::CapabilityLabel,
            Message::PromptLabel,
            Message::DelegateAgentCapability,
            Message::DelegateAgentConfirmationRequired,
            Message::NetworkAskNonInteractive,
            Message::NetworkAskPrompt,
            Message::NetworkAllowedByUser,
            Message::NetworkDeniedByUser,
        ];
        for language in languages {
            for message in messages {
                let text = text_for(language, message);
                assert!(!text.trim().is_empty(), "{language:?} {message:?}");
            }
        }
        assert_eq!(text_for(Language::ZhHans, Message::TaskLabel), "任务：");
        assert_eq!(
            text_for(Language::En, Message::NetworkAskPrompt),
            "Allow model network request now? [y/N]: "
        );
    }

    #[test]
    fn localizes_interactive_debug_and_compaction_surface() {
        let messages = [
            Message::InteractiveWelcomeHint,
            Message::InteractiveContextLabel,
            Message::InteractiveSlashCompactHint,
            Message::InteractiveCompactCompleted,
            Message::InteractiveCompactHistoryPreserved,
        ];
        for message in messages {
            assert!(!text_for(Language::En, message).trim().is_empty());
            assert!(!text_for(Language::ZhHans, message).trim().is_empty());
        }
        assert_eq!(
            text_for(Language::ZhHans, Message::InteractiveContextLabel),
            "上下文"
        );
        assert_eq!(
            text_for(Language::ZhHans, Message::InteractiveCompactCompleted),
            "压缩完成"
        );
    }

    #[test]
    fn every_help_locale_exposes_the_complete_cli_surface() {
        let languages = [
            Language::ZhHans,
            Language::En,
            Language::De,
            Language::Es,
            Language::Fr,
            Language::Ja,
            Language::Ru,
        ];
        let required_commands = [
            "nola init",
            "nola wiki",
            "nola web",
            "nola sessions",
            "nola session show",
            "nola session delete",
        ];
        let required_options = [
            "--endpoint",
            "--project",
            "--debug",
            "--network-policy",
            "--force-tool-set",
            "--resume",
            "--output",
            "--dry-run",
            "--host",
            "--port",
            "--web-root",
        ];
        let required_environment = [
            "NOLA_AGENT_CONTEXT_WINDOW_TOKENS",
            "NOLA_CLI_DEBUG_LOG_PATH",
            "NOLA_CLI_INTERACTION_MODE",
            "NOLA_DB_PATH",
        ];

        for language in languages {
            let help = super::help_text_for(language);
            for needle in required_commands
                .iter()
                .chain(required_options.iter())
                .chain(required_environment.iter())
            {
                assert!(
                    help.contains(needle),
                    "{language:?} help is missing {needle}"
                );
            }
        }
    }

    #[test]
    fn localizes_common_tool_error_display_text() {
        assert_eq!(
            localize_tool_error_display_for(Language::ZhHans, "Not a file: README.md"),
            "不是文件：README.md"
        );
        assert_eq!(
            localize_tool_error_display_for(
                Language::De,
                "path is outside allowed directories: /tmp/x"
            ),
            "Pfad liegt außerhalb der erlaubten Verzeichnisse: /tmp/x"
        );
        assert_eq!(
            localize_tool_error_display_for(Language::Ja, "old_text was not found"),
            "old_text が見つかりません"
        );
    }
}
