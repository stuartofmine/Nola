use std::env;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    ZhHans,
    ZhHant,
    En,
    De,
    Es,
    Fr,
    Ja,
    Ko,
    Ru,
}

pub(crate) fn new_conversation_title(language: Language) -> &'static str {
    match language {
        Language::ZhHans => "新对话",
        Language::ZhHant => "新對話",
        Language::En => "New conversation",
        Language::De => "Neue Unterhaltung",
        Language::Es => "Nueva conversación",
        Language::Fr => "Nouvelle conversation",
        Language::Ja => "新しい会話",
        Language::Ko => "새 대화",
        Language::Ru => "Новый диалог",
    }
}

pub(crate) fn cli_view_state_save_failed(error: &str) -> String {
    match current_language() {
        Language::ZhHans => format!("无法保存 CLI 显示状态：{error}"),
        Language::ZhHant => format!("無法儲存 CLI 顯示狀態：{error}"),
        Language::De => format!("CLI-Anzeigestatus konnte nicht gespeichert werden: {error}"),
        Language::Es => format!("No se pudo guardar el estado de vista de la CLI: {error}"),
        Language::Fr => format!("Impossible d'enregistrer l'état d'affichage CLI : {error}"),
        Language::Ja => format!("CLI 表示状態を保存できませんでした: {error}"),
        Language::Ko => format!("CLI 표시 상태를 저장할 수 없습니다: {error}"),
        Language::Ru => format!("Не удалось сохранить состояние отображения CLI: {error}"),
        Language::En => format!("Unable to save CLI view state: {error}"),
    }
}

pub(crate) fn cli_saved_viewport_unavailable() -> &'static str {
    match current_language() {
        Language::ZhHans => "原阅读内容已不可用；草稿已保留，当前显示最近记录。",
        Language::ZhHant => "原閱讀內容已不可用；草稿已保留，目前顯示最近紀錄。",
        Language::De => "Vorherige Leseposition nicht mehr verfügbar; Entwurf bleibt erhalten, zeige aktuellen Verlauf.",
        Language::Es => "La vista de lectura anterior ya no está disponible; se conserva el borrador y se muestra el historial reciente.",
        Language::Fr => "La position de lecture précédente n'est plus disponible ; le brouillon est conservé et l'historique récent est affiché.",
        Language::Ja => "以前の閲覧位置は利用できません。下書きは保持され、最近の履歴を表示しています。",
        Language::Ko => "이전 읽기 위치를 사용할 수 없습니다. 초안은 유지되며 최근 기록을 표시합니다.",
        Language::Ru => "Предыдущая позиция чтения недоступна; черновик сохранён, показана последняя история.",
        Language::En => "Previous reading position is no longer available; draft preserved, showing recent history.",
    }
}

pub(crate) fn cli_recovery_incomplete_hint() -> &'static str {
    match current_language() {
        Language::ZhHans => "恢复尚未完成；可用 /resume 重试或切换会话，/exit 退出。原请求已保留。",
        Language::ZhHant => "恢復尚未完成；可用 /resume 重試或切換工作階段，/exit 退出。原請求已保留。",
        Language::De => "Wiederherstellung unvollständig; nutzen Sie /resume zum Wiederholen oder Wechseln, /exit zum Beenden. Ursprüngliche Anfrage bleibt erhalten.",
        Language::Es => "Recuperación incompleta; usa /resume para reintentar o cambiar de sesión, o /exit para salir. La solicitud original se conserva.",
        Language::Fr => "Reprise incomplète ; utilisez /resume pour réessayer ou changer de session, /exit pour quitter. La demande initiale est conservée.",
        Language::Ja => "復元の途中です。/resume で再試行またはセッション切替、/exit で終了できます。元のリクエストは保持されています。",
        Language::Ko => "복구가 완료되지 않았습니다. /resume 으로 재시도하거나 세션을 전환하고, /exit 으로 종료할 수 있습니다. 원래 요청은 유지됩니다.",
        Language::Ru => "Восстановление не завершено; используйте /resume для повтора или смены сессии, /exit для выхода. Исходный запрос сохранён.",
        Language::En => "Recovery is incomplete; use /resume to retry or switch sessions, or /exit to quit. Original request preserved.",
    }
}

pub(crate) fn cli_stopping_to_exit() -> &'static str {
    match current_language() {
        Language::ZhHans => "正在停止当前执行并退出…",
        Language::ZhHant => "正在停止目前執行並退出…",
        Language::De => "Aktuelle Ausführung wird zum Beenden gestoppt…",
        Language::Es => "Deteniendo la ejecución actual para salir…",
        Language::Fr => "Arrêt de l’exécution en cours avant de quitter…",
        Language::Ja => "現在の実行を停止して終了しています…",
        Language::Ko => "현재 실행을 중지하고 종료하는 중…",
        Language::Ru => "Остановка текущего выполнения перед выходом…",
        Language::En => "Stopping current execution to exit…",
    }
}

pub(crate) fn cli_stopping_to_resume() -> &'static str {
    match current_language() {
        Language::ZhHans => "正在停止当前执行以切换会话…",
        Language::ZhHant => "正在停止目前執行以切換工作階段…",
        Language::De => "Aktuelle Ausführung wird zum Fortsetzen gestoppt…",
        Language::Es => "Deteniendo la ejecución actual para reanudar…",
        Language::Fr => "Arrêt de l’exécution en cours pour reprendre…",
        Language::Ja => "現在の実行を停止して再開しています…",
        Language::Ko => "현재 실행을 중지하고 세션을 재개하는 중…",
        Language::Ru => "Остановка текущего выполнения для возобновления…",
        Language::En => "Stopping current execution to resume…",
    }
}

pub(crate) fn cli_view_state_restore_failed(error: &str) -> String {
    match current_language() {
        Language::ZhHans => format!("无法恢复 CLI 显示状态：{error}"),
        Language::ZhHant => format!("無法恢復 CLI 顯示狀態：{error}"),
        Language::De => format!("CLI-Anzeigestatus konnte nicht wiederhergestellt werden: {error}"),
        Language::Es => format!("No se pudo restaurar el estado de vista de la CLI: {error}"),
        Language::Fr => format!("Impossible de restaurer l’état d’affichage CLI : {error}"),
        Language::Ja => format!("CLI 表示状態を復元できませんでした: {error}"),
        Language::Ko => format!("CLI 표시 상태를 복원할 수 없습니다: {error}"),
        Language::Ru => format!("Не удалось восстановить состояние отображения CLI: {error}"),
        Language::En => format!("Unable to restore CLI view state: {error}"),
    }
}

pub(crate) fn cli_command_unavailable_in_context() -> &'static str {
    match current_language() {
        Language::ZhHans => "当前上下文中不可使用该命令。",
        Language::ZhHant => "目前上下文中無法使用該指令。",
        Language::De => "Dieser Befehl ist im aktuellen Kontext nicht verfügbar.",
        Language::Es => "Este comando no está disponible en el contexto actual.",
        Language::Fr => "Cette commande n’est pas disponible dans le contexte actuel.",
        Language::Ja => "このコマンドは現在のコンテキストでは使用できません。",
        Language::Ko => "현재 컨텍스트에서는 이 명령을 사용할 수 없습니다.",
        Language::Ru => "Эта команда недоступна в текущем контексте.",
        Language::En => "This command is not available in the current context.",
    }
}

pub(crate) fn cli_pasted_content_badge(chars: usize) -> String {
    match current_language() {
        Language::ZhHans => format!("[已粘贴内容：{chars} 字符]"),
        Language::ZhHant => format!("[已貼上內容：{chars} 字元]"),
        Language::De => format!("[Eingefügter Inhalt: {chars} Zeichen]"),
        Language::Es => format!("[Contenido pegado: {chars} caracteres]"),
        Language::Fr => format!("[Contenu collé : {chars} caractères]"),
        Language::Ja => format!("[貼り付け内容: {chars} 文字]"),
        Language::Ko => format!("[붙여넣은 내용: {chars}자]"),
        Language::Ru => format!("[Вставлено: {chars} симв.]"),
        Language::En => format!("[Pasted content: {chars} chars]"),
    }
}

pub(crate) fn cli_section_label(key: &'static str) -> &'static str {
    let values = match key {
        "last response" => [
            "上次回复",
            "上次回覆",
            "last response",
            "Letzte Antwort",
            "Última respuesta",
            "Dernière réponse",
            "前回の応答",
            "마지막 응답",
            "Последний ответ",
        ],
        "continue" => [
            "继续",
            "繼續",
            "continue",
            "Fortsetzen",
            "Continuar",
            "Continuer",
            "続行",
            "계속",
            "Продолжить",
        ],
        "state" => [
            "状态",
            "狀態",
            "state",
            "Status",
            "Estado",
            "État",
            "状態",
            "상태",
            "Состояние",
        ],
        "init" => [
            "初始化",
            "初始化",
            "init",
            "Initialisierung",
            "Inicialización",
            "Initialisation",
            "初期化",
            "초기화",
            "Инициализация",
        ],
        "wiki" => [
            "知识库",
            "知識庫",
            "wiki",
            "Wiki",
            "Wiki",
            "Wiki",
            "Wiki",
            "위키",
            "Вики",
        ],
        "plan" => [
            "计划",
            "計畫",
            "plan",
            "Plan",
            "Plan",
            "Plan",
            "計画",
            "계획",
            "План",
        ],
        "plan history" => [
            "计划历史",
            "計畫歷史",
            "plan history",
            "Planverlauf",
            "Historial del plan",
            "Historique du plan",
            "計画履歴",
            "계획 기록",
            "История плана",
        ],
        "plan diff" => [
            "计划差异",
            "計畫差異",
            "plan diff",
            "Planänderungen",
            "Diferencias del plan",
            "Différences du plan",
            "計画の差分",
            "계획 변경 사항",
            "Изменения плана",
        ],
        "skills" => [
            "技能",
            "技能",
            "skills",
            "Skills",
            "Skills",
            "Skills",
            "スキル",
            "스킬",
            "Навыки",
        ],
        "mcp" => [
            "MCP",
            "MCP",
            "mcp",
            "MCP",
            "MCP",
            "MCP",
            "MCP",
            "MCP",
            "MCP",
        ],
        "guide" => [
            "指南",
            "指南",
            "guide",
            "Leitfaden",
            "Guía",
            "Guide",
            "ガイド",
            "가이드",
            "Руководство",
        ],
        _ => return key,
    };
    pick_language(current_language(), values)
}

pub(crate) fn cli_executor_status_label(status: &'static str) -> &'static str {
    let values = match status {
        "available" => [
            "可用",
            "可用",
            "available",
            "verfügbar",
            "disponible",
            "disponible",
            "利用可能",
            "사용 가능",
            "доступно",
        ],
        "not installed" => [
            "未安装",
            "未安裝",
            "not installed",
            "nicht installiert",
            "no instalado",
            "non installé",
            "未インストール",
            "설치되지 않음",
            "не установлено",
        ],
        "not authenticated" => [
            "未认证",
            "未驗證",
            "not authenticated",
            "nicht authentifiziert",
            "no autenticado",
            "non authentifié",
            "未認証",
            "인증되지 않음",
            "не аутентифицировано",
        ],
        "incompatible" => [
            "不兼容",
            "不相容",
            "incompatible",
            "inkompatibel",
            "incompatible",
            "incompatible",
            "非互換",
            "호환되지 않음",
            "несовместимо",
        ],
        "unreachable" => [
            "不可达",
            "無法連線",
            "unreachable",
            "nicht erreichbar",
            "inalcanzable",
            "injoignable",
            "到達不能",
            "연결할 수 없음",
            "недоступно",
        ],
        _ => return status,
    };
    pick_language(current_language(), values)
}

pub(crate) fn cli_history_empty() -> &'static str {
    match current_language() {
        Language::ZhHans => "当前会话暂无历史记录。",
        Language::ZhHant => "目前工作階段尚無歷史紀錄。",
        Language::De => "Keine Verlaufsnachrichten in dieser Sitzung.",
        Language::Es => "No hay historial en esta sesión.",
        Language::Fr => "Aucun historique dans cette session.",
        Language::Ja => "このセッションには履歴がありません。",
        Language::Ko => "이 세션에는 기록이 없습니다.",
        Language::Ru => "В этой сессии пока нет истории.",
        Language::En => "No history in this session.",
    }
}

pub(crate) fn pick_language(language: Language, values: [&'static str; 9]) -> &'static str {
    match language {
        Language::ZhHans => values[0],
        Language::ZhHant => values[1],
        Language::En => values[2],
        Language::De => values[3],
        Language::Es => values[4],
        Language::Fr => values[5],
        Language::Ja => values[6],
        Language::Ko => values[7],
        Language::Ru => values[8],
    }
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
    AbsentSessionPrefix,
    FailedReadPromptPrefix,
    PromptRequired,
    SessionRequiresAction,
    RequiresSessionIdSuffix,
    UnknownSessionActionPrefix,
    UnknownCommandPrefix,
    MaxIterationsPositive,
    UnknownOptionPrefix,
    UnexpectedCommandArgs,
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
    InteractiveGoalControlPending,
    InteractiveGoalAlreadyActive,
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

static LANGUAGE_OVERRIDE: RwLock<Option<Language>> = RwLock::new(None);

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
        Language::ZhHant => PermissionText {
            allow_once_option: "僅允許本次",
            deny_option: "拒絕",
            keyboard_hint: "使用 ↑/↓ 選擇，Enter 確認 · 也可按 1/2/3/N",
            once_only_keyboard_hint: "使用 ↑/↓ 選擇，Enter 確認 · 也可按 1/3/N",
            session_unavailable_option: "本對話允許（目前請求未提供此選項）",
            custom_response: "返回主輸入框…",
            allow_conversation_prefix: "本對話允許：",
            file_access_title: "檔案存取需要授權",
            default_working_directory: "預設工作目錄",
            cwd: "工作目錄",
            command: "指令",
            reason: "原因",
            endpoint: "端點",
            model: "模型",
            path: "路徑",
            directory: "目錄",
            allowed: "已允許",
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
        Language::Ko => PermissionText {
            allow_once_option: "한 번만 허용",
            deny_option: "거부",
            keyboard_hint: "↑/↓로 선택 후 Enter로 확인 · 1/2/3/N 사용 가능",
            once_only_keyboard_hint: "↑/↓로 선택 후 Enter로 확인 · 1/3/N 사용 가능",
            session_unavailable_option: "이 세션에서 허용(이 요청에서는 선택할 수 없음)",
            custom_response: "입력창으로 돌아가기…",
            allow_conversation_prefix: "이 세션에서 허용:",
            file_access_title: "파일 접근 권한이 필요합니다",
            default_working_directory: "기본 작업 디렉터리",
            cwd: "작업 위치",
            command: "명령어",
            reason: "이유",
            endpoint: "엔드포인트",
            model: "모델",
            path: "경로",
            directory: "디렉터리",
            allowed: "허용됨",
            tool: "도구",
            summary: "요약",
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
    MissingDefaultModel,
    DefaultModelChoices,
    ApplyConfigAgain,
    ApplyConfigHelp,
    FileHelpShow,
    FileHelpEdit,
    FileHelpCheck,
    FileHelpPreview,
    FileHelpApply,
    FileHelpExample,
    FileOverview,
    FileAutomatic,
    FileDisabled,
    FileHostUnavailable,
    FileHostDefault,
    FileNone,
    FileLoadOrder,
    FileSettings,
    FileModelSource,
    FileDeclaredDefault,
    FileApplied,
    FileStartup,
    FileSkipped,
    FilePreview,
    FileChecked,
    FileRouteModel,
    FileRuntimeOnly,
    FileNotSaved,
    LocateModelConfig,
    CheckModelConfigFile,
    CheckModelConfigSize,
    CheckModelConfigJson,
    AddModelConfigLlm,
    PermissionDigitHint,
    PermissionScrollHint,
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
    GoalIntro,
    GoalInputHint,
    SlashMenuKeys,
    MentionPickKeys,
    HelpMoreCommands,
    HelpEdit,
    HelpCancelEdit,
    EditInputHint,
    NoActiveEdit,
    NoMessageToEdit,
    GoalPlaceholder,
    GoalLifecycle,
    Constraints,
    DefaultSafeConstraints,
    Mode,
    AutonomousUntilComplete,
    NoAvailableModels,
    HelpAgent,
    Work,
    WorkUsage,
    WorkResolveHint,
    HelpRoom,
    NoRooms,
    NoPendingQuestions,
    QuestionKeyHint,
    QuestionLineHint,
    QuestionDeferred,
    DeclineQuestion,
    PermissionInstructionOption,
    PermissionInstructionCommand,
    NoConfiguredMcp,
    NoAvailableSkills,
    MemoryHeading,
    MemorySearchHeading,
    MemorySaved,
    NoSavedMemory,
    NoMatchingMemory,
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
    RecoveryBlockedUseResume,
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
    EndpointInvalidScheme,
    ModelPrompt,
    ModelRequired,
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
        UiText::MissingDefaultModel => [
            "llm.default_model 指定的连接和模型组合不存在。", "llm.default_model 指定的連線與模型組合不存在。",
            "default_model must refer to a declared connection and model.", "Die Verbindung und das Modell in llm.default_model sind nicht zusammen konfiguriert.",
            "La combinación de conexión y modelo en llm.default_model no está configurada.", "La connexion et le modèle de llm.default_model ne sont pas configurés ensemble.",
            "llm.default_model で指定した接続とモデルの組み合わせがありません。", "llm.default_model에 지정한 연결과 모델 조합이 없습니다.",
            "В llm.default_model указана неизвестная пара подключения и модели.",
        ],
        UiText::DefaultModelChoices => [
            "请从 llm.connections 选择一组 ID 填入 llm.default_model，例如", "請從 llm.connections 選一組 ID 填入 llm.default_model，例如",
            "Set llm.default_model using connection/model IDs from llm.connections, for example:", "Setze llm.default_model auf ein ID-Paar aus llm.connections, zum Beispiel:",
            "Asigna a llm.default_model un par de identificadores de llm.connections, por ejemplo:", "Renseignez llm.default_model avec deux identifiants de llm.connections, par exemple :",
            "llm.connections の ID の組を llm.default_model に設定してください。例", "llm.connections의 ID 조합을 llm.default_model에 입력하세요. 예시",
            "Для llm.default_model выберите пару идентификаторов из llm.connections, например:",
        ],
        UiText::FileHelpShow => ["查看配置加载顺序和文件声明的模型。", "查看設定載入順序和檔案宣告的模型。", "Show configuration files in load order and the model they declare.", "Konfigurationsdateien in Ladereihenfolge und Modell anzeigen.", "Mostrar los archivos en orden de carga y el modelo declarado.", "Afficher les fichiers dans leur ordre de chargement et le modèle déclaré.", "設定ファイルの読み込み順と宣言されたモデルを表示。", "설정 파일의 로드 순서와 선언된 모델을 표시합니다.", "Показать порядок загрузки файлов и объявленную модель."],
        UiText::FileHelpEdit => ["用编辑器打开配置文件。", "用編輯器開啟設定檔。", "Open a configuration file in your editor.", "Konfigurationsdatei im Editor öffnen.", "Abrir un archivo de configuración en el editor.", "Ouvrir un fichier de configuration dans l’éditeur.", "設定ファイルをエディターで開く。", "편집기로 설정 파일을 엽니다.", "Открыть файл конфигурации в редакторе."],
        UiText::FileHelpCheck => ["检查启动配置，或单独检查指定文件。", "檢查啟動設定，或單獨檢查指定檔案。", "Validate the startup layers or one file.", "Startkonfiguration oder eine Datei prüfen.", "Validar la configuración de inicio o un archivo.", "Vérifier la configuration de démarrage ou un fichier.", "起動設定または指定ファイルを検証。", "시작 설정 또는 지정한 파일을 검사합니다.", "Проверить настройки запуска или отдельный файл."],
        UiText::FileHelpPreview => ["预览文件要写入宿主的连接和默认模型。", "預覽檔案要寫入主機的連線和預設模型。", "Preview the connections and Host default declared by the file.", "Deklarierte Verbindungen und Host-Standardmodell anzeigen.", "Previsualizar las conexiones y el modelo predeterminado declarados.", "Prévisualiser les connexions et le modèle par défaut déclarés.", "接続とホストの既定モデルの設定をプレビュー。", "연결과 호스트 기본 모델 설정을 미리 봅니다.", "Предпросмотр подключений и модели хоста по умолчанию."],
        UiText::FileHelpApply => ["立即应用（可选）。", "立即套用（可選）。", "Apply immediately (optional).", "Sofort anwenden (optional).", "Aplicar ahora (opcional).", "Appliquer maintenant (facultatif).", "今すぐ適用（任意）。", "즉시 적용합니다(선택 사항).", "Применить сразу (необязательно)."],
        UiText::FileHelpExample => ["输出可编辑的模型目录 JSON 示例。", "輸出可編輯的模型目錄 JSON 範例。", "Print an editable model catalog as JSON.", "Bearbeitbares JSON-Beispiel ausgeben.", "Mostrar un catálogo de ejemplo en JSON.", "Afficher un exemple de catalogue JSON à modifier.", "編集可能な JSON カタログの例を出力。", "편집할 수 있는 JSON 카탈로그 예제를 출력합니다.", "Вывести пример каталога в JSON для редактирования."],
        UiText::FileOverview => ["查看配置文件和模型来源", "查看設定檔案和模型來源", "Show configuration files and model source", "Konfiguration und Modellquelle anzeigen", "Ver archivos de configuración y origen del modelo", "Voir les fichiers de configuration et la source du modèle", "設定ファイルとモデルの参照元を表示", "설정 파일과 모델 출처 보기", "Показать файлы конфигурации и источник модели"],
        UiText::FileAutomatic => ["自动选择", "自動選擇", "Automatic", "Automatisch", "Automático", "Automatique", "自動", "자동", "Автоматически"],
        UiText::FileDisabled => ["（已停用）", "（已停用）", " (disabled)", " (deaktiviert)", " (deshabilitado)", " (désactivé)", "（無効）", " (사용 안 함)", " (отключена)"],
        UiText::FileHostUnavailable => ["不可用 · nola model connections", "無法使用 · nola model connections", "Unavailable · nola model connections", "Nicht verfügbar · nola model connections", "No disponible · nola model connections", "Indisponible · nola model connections", "利用不可 · nola model connections", "사용할 수 없음 · nola model connections", "Недоступно · nola model connections"],
        UiText::FileHostDefault => ["宿主已保存的默认模型", "主機已儲存的預設模型", "Saved Host default", "Gespeichertes Host-Standardmodell", "Modelo predeterminado guardado en el host", "Modèle par défaut enregistré sur l’hôte", "ホストに保存された既定モデル", "호스트에 저장된 기본 모델", "Сохранённая модель хоста"],
        UiText::FileNone => ["未找到配置文件。", "找不到設定檔。", "No configuration files found.", "Keine Konfigurationsdateien gefunden.", "No se encontraron archivos de configuración.", "Aucun fichier de configuration trouvé.", "設定ファイルが見つかりません。", "설정 파일을 찾을 수 없습니다.", "Файлы конфигурации не найдены."],
        UiText::FileLoadOrder => ["配置文件（按加载顺序）", "設定檔（依載入順序）", "Files in startup order", "Dateien in Ladereihenfolge", "Archivos en orden de carga", "Fichiers par ordre de chargement", "読み込み順の設定ファイル", "로드 순서에 따른 설정 파일", "Файлы в порядке загрузки"],
        UiText::FileSettings => ["文件运行设置", "檔案執行設定", "File settings", "Einstellungen aus Dateien", "Ajustes de los archivos", "Paramètres des fichiers", "ファイルの設定", "파일 설정", "Параметры из файлов"],
        UiText::FileModelSource => ["模型目录", "模型目錄", "Model catalog", "Modellkatalog", "Catálogo de modelos", "Catalogue de modèles", "モデルカタログ", "모델 카탈로그", "Каталог моделей"],
        UiText::FileDeclaredDefault => ["文件声明的宿主默认模型", "檔案宣告的主機預設模型", "Declared Host default", "Deklariertes Host-Standardmodell", "Modelo predeterminado declarado", "Modèle par défaut déclaré", "ファイルで宣言した既定モデル", "파일에 선언된 기본 모델", "Модель по умолчанию в файле"],
        UiText::FileStartup => ["下次启动自动应用。", "下次啟動時自動套用。", "Applied automatically on startup.", "Wird beim Start automatisch angewendet.", "Se aplica automáticamente al iniciar.", "Application automatique au démarrage.", "起動時に自動適用します。", "시작할 때 자동으로 적용합니다.", "Применяется автоматически при запуске."],
        UiText::FileSkipped => ["已跳过模型配置", "已略過模型設定", "Skipped model configuration", "Modellkonfiguration übersprungen", "Configuración de modelos omitida", "Configuration de modèles ignorée", "モデル設定をスキップしました", "모델 설정을 건너뜁니다", "Конфигурация моделей пропущена"],
        UiText::FileApplied => ["已保存到这台宿主。", "已儲存到這台主機。", "Saved to this Host.", "Auf diesem Host gespeichert.", "Guardado en este host.", "Enregistré sur cet hôte.", "このホストに保存しました。", "이 호스트에 저장했습니다.", "Сохранено на этом хосте."],
        UiText::FilePreview => ["仅预览文件声明。应用时还会核对已有连接和凭据。", "僅預覽檔案宣告。套用時也會核對現有連線和憑證。", "Preview only. Apply also checks stored connections and credentials.", "Vorschau. Beim Anwenden werden bestehende Verbindungen und Zugangsdaten geprüft.", "Vista previa. Al aplicar se comprueban las conexiones y credenciales existentes.", "Aperçu. L’application vérifie aussi les connexions et identifiants existants.", "プレビューです。適用時に既存の接続と認証情報を確認します。", "미리보기입니다. 적용 시 기존 연결과 인증 정보를 확인합니다.", "Предпросмотр. При применении проверяются существующие подключения и учётные данные."],
        UiText::FileChecked => ["文件配置检查通过。", "檔案設定檢查通過。", "File configuration is valid.", "Dateikonfiguration gültig.", "Configuración de archivo válida.", "Configuration du fichier valide.", "ファイルの設定は有効です。", "파일 설정이 유효합니다.", "Настройки файла прошли проверку."],
        UiText::FileRouteModel => ["直连模型", "直連模型", "Route model", "Direktes Modell", "Modelo de conexión directa", "Modèle en connexion directe", "直接接続のモデル", "직접 연결 모델", "Модель прямого подключения"],
        UiText::FileRuntimeOnly => ["运行设置在启动时读取。这组文件无需导入模型目录。", "執行設定在啟動時讀取。這組檔案無須匯入模型目錄。", "Runtime settings take effect at startup. This file set has no catalog to publish.", "Laufzeiteinstellungen gelten ab dem Start. Diese Dateien enthalten keinen zu importierenden Katalog.", "Los ajustes se leen al iniciar. Estos archivos no declaran un catálogo para importar.", "Les paramètres sont lus au démarrage. Ces fichiers ne déclarent aucun catalogue à importer.", "実行設定は起動時に読み込まれます。インポートするカタログはありません。", "실행 설정은 시작할 때 읽습니다. 가져올 카탈로그가 없습니다.", "Настройки читаются при запуске. В этих файлах нет каталога для импорта."],
        UiText::FileNotSaved => ["未保存文件。", "未儲存檔案。", "No file saved.", "Keine Datei gespeichert.", "No se guardó ningún archivo.", "Aucun fichier enregistré.", "ファイルは保存されませんでした。", "파일을 저장하지 않았습니다.", "Файл не сохранён."],
        UiText::ApplyConfigHelp => [
            "读取指定的 JSON 或 JSONC 文件，保存 llm.connections 并选择 llm.default_model。省略路径时读取 ~/.nola/config.json 或 ~/.nola/config.jsonc。",
            "讀取指定的 JSON 或 JSONC 檔案，儲存 llm.connections 並選取 llm.default_model。省略路徑時讀取 ~/.nola/config.json 或 ~/.nola/config.jsonc。",
            "Save llm.connections and select llm.default_model from a JSON or JSONC file. The default file is ~/.nola/config.json or ~/.nola/config.jsonc.",
            "llm.connections speichern und llm.default_model aus einer JSON- oder JSONC-Datei auswählen. Standarddatei: ~/.nola/config.json oder ~/.nola/config.jsonc.",
            "Guarda llm.connections y selecciona llm.default_model desde JSON o JSONC. Por defecto se usa ~/.nola/config.json o ~/.nola/config.jsonc.",
            "Enregistre llm.connections et sélectionne llm.default_model depuis un fichier JSON ou JSONC. Par défaut, utilise ~/.nola/config.json ou ~/.nola/config.jsonc.",
            "JSON または JSONC から llm.connections を保存し、llm.default_model を選択します。既定のファイルは ~/.nola/config.json または ~/.nola/config.jsonc です。",
            "JSON 또는 JSONC 파일에서 llm.connections를 저장하고 llm.default_model을 선택합니다. 기본 파일은 ~/.nola/config.json 또는 ~/.nola/config.jsonc입니다.",
            "Сохраняет llm.connections и выбирает llm.default_model из файла JSON или JSONC. По умолчанию используется ~/.nola/config.json или ~/.nola/config.jsonc.",
        ],
        UiText::LocateModelConfig => [
            "未找到默认配置。请指定一个 JSON 或 JSONC 配置文件：",
            "找不到預設設定。請指定一個 JSON 或 JSONC 設定檔：",
            "No default configuration found. Specify a JSON or JSONC configuration file:",
            "Keine Standardkonfiguration gefunden. Eine JSON- oder JSONC-Datei angeben:",
            "No se encontró la configuración predeterminada. Indica un archivo JSON o JSONC:",
            "Configuration par défaut introuvable. Indiquez un fichier JSON ou JSONC :",
            "既定の設定が見つかりません。JSON または JSONC の設定ファイルを指定してください。",
            "기본 설정이 없습니다. JSON 또는 JSONC 설정 파일을 지정하세요.",
            "Конфигурация по умолчанию не найдена. Укажите файл JSON или JSONC:",
        ],
        UiText::CheckModelConfigFile => [
            "请检查文件路径和读取权限。",
            "請檢查檔案路徑與讀取權限。",
            "Check the file path and read permissions.",
            "Dateipfad und Leserechte prüfen.",
            "Comprueba la ruta y los permisos de lectura.",
            "Vérifiez le chemin et les droits de lecture.",
            "ファイルのパスと読み取り権限を確認してください。",
            "파일 경로와 읽기 권한을 확인하세요.",
            "Проверьте путь к файлу и права чтения.",
        ],
        UiText::CheckModelConfigSize => [
            "请选择不超过 2 MiB 的普通文件。",
            "請選擇不超過 2 MiB 的一般檔案。",
            "Choose a regular file no larger than 2 MiB.",
            "Eine reguläre Datei von höchstens 2 MiB wählen.",
            "Selecciona un archivo normal de hasta 2 MiB.",
            "Choisissez un fichier ordinaire de 2 Mio au maximum.",
            "2 MiB 以下の通常ファイルを選択してください。",
            "2 MiB 이하의 일반 파일을 선택하세요.",
            "Выберите обычный файл размером не более 2 МиБ.",
        ],
        UiText::CheckModelConfigJson => [
            "请检查文件中的 JSON 或 JSONC 语法。",
            "請檢查檔案中的 JSON 或 JSONC 語法。",
            "Check the JSON or JSONC syntax in this file.",
            "Die JSON- oder JSONC-Syntax in dieser Datei prüfen.",
            "Revisa la sintaxis JSON o JSONC del archivo.",
            "Vérifiez la syntaxe JSON ou JSONC du fichier.",
            "このファイルの JSON または JSONC 構文を確認してください。",
            "파일의 JSON 또는 JSONC 구문을 확인하세요.",
            "Проверьте синтаксис JSON или JSONC в файле.",
        ],
        UiText::AddModelConfigLlm => [
            "请在文件顶层添加 llm 对象。",
            "請在檔案頂層加入 llm 物件。",
            "missing llm object at the top level of this file",
            "Das Objekt llm auf der obersten Ebene der Datei ergänzen.",
            "Añade el objeto llm en la raíz del archivo.",
            "Ajoutez l’objet llm à la racine du fichier.",
            "ファイルの最上位に llm オブジェクトを追加してください。",
            "파일의 최상위에 llm 객체를 추가하세요.",
            "Добавьте объект llm на верхнем уровне файла.",
        ],
        UiText::ApplyConfigAgain => [
            "保存配置后重新运行", "儲存設定後重新執行", "Save the configuration, then rerun", "Konfiguration speichern und erneut ausführen",
            "Guarda la configuración y vuelve a ejecutar", "Enregistrez la configuration et relancez", "設定を保存し、次のコマンドを再実行してください。",
            "설정을 저장한 후 다시 실행하세요.", "Сохраните конфигурацию и повторите команду",
        ],
        UiText::PermissionDigitHint => ["↑↓ 或 1/2 选择 · Enter 应用 · Esc 取消", "↑↓ 或 1/2 選擇 · Enter 套用 · Esc 取消", "↑↓ or 1/2 select · Enter applies · Esc cancels", "↑↓ oder 1/2 wählen · Enter übernehmen · Esc abbrechen", "↑↓ o 1/2 elegir · Intro aplicar · Esc cancelar", "↑↓ ou 1/2 choisir · Entrée appliquer · Échap annuler", "↑↓ または 1/2 選択 · Enter 適用 · Esc キャンセル", "↑↓ 또는 1/2 선택 · Enter 적용 · Esc 취소", "↑↓ или 1/2 выбрать · Enter применить · Esc отмена"],
        UiText::PermissionScrollHint => ["PgUp/PgDn 翻页 · ↑/↓ 选择", "PgUp/PgDn 翻頁 · ↑/↓ 選擇", "PgUp/PgDn: details · ↑/↓: select", "PgUp/PgDn blättern · ↑/↓ wählen", "PgUp/PgDn detalles · ↑/↓ elegir", "PgUp/PgDn détails · ↑/↓ choisir", "PgUp/PgDn 詳細 · ↑/↓ 選択", "PgUp/PgDn 상세 · ↑/↓ 선택", "PgUp/PgDn детали · ↑/↓ выбор"],
        UiText::HelpSources => [
            "管理数据源：/sources [list|add|test|remove|enable|disable|inherit]",
            "管理資料來源：/sources [list|add|test|remove|enable|disable|inherit]",
            "Manage data sources: /sources [list|add|test|remove|enable|disable|inherit]",
            "Datenquellen verwalten: /sources [list|add|test|remove|enable|disable|inherit]",
            "Gestionar fuentes de datos: /sources [list|add|test|remove|enable|disable|inherit]",
            "Gérer les sources de données : /sources [list|add|test|remove|enable|disable|inherit]",
            "データソースを管理: /sources [list|add|test|remove|enable|disable|inherit]",
            "데이터 소스 관리: /sources [list|add|test|remove|enable|disable|inherit]",
            "Управление источниками данных: /sources [list|add|test|remove|enable|disable|inherit]",
        ],
        UiText::DataSourcesTitle => ["数据源", "資料來源", "Data sources", "Datenquellen", "Fuentes de datos", "Sources de données", "データソース", "데이터 소스", "Источники данных"],
        UiText::DataSourcesUsage => ["用法", "用法", "Usage", "Verwendung", "Uso", "Utilisation", "使い方", "사용법", "Использование"],
        UiText::DataSourceAdded => ["数据源已测试并添加", "資料來源已測試並新增", "Data source tested and added", "Datenquelle getestet und hinzugefügt", "Fuente de datos probada y añadida", "Source de données testée et ajoutée", "データソースをテストして追加しました", "데이터 소스를 테스트하고 추가했습니다", "Источник данных проверен и добавлен"],
        UiText::DataSourceTested => ["数据源测试成功", "資料來源測試成功", "Data source test passed", "Datenquellentest erfolgreich", "Prueba de fuente de datos superada", "Test de la source de données réussi", "データソースのテストに成功しました", "데이터 소스 테스트 통과", "Проверка источника данных пройдена"],
        UiText::DataSourceRemoved => ["数据源已删除", "資料來源已移除", "Data source removed", "Datenquelle entfernt", "Fuente de datos eliminada", "Source de données supprimée", "データソースを削除しました", "데이터 소스를 제거했습니다", "Источник данных удалён"],
        UiText::DataSourceEnabled => ["已在当前会话启用", "已在此對話啟用", "Enabled for this conversation", "Für diese Unterhaltung aktiviert", "Activada para esta conversación", "Activée pour cette conversation", "この会話で有効にしました", "이 대화에서 사용 설정됨", "Включено для этого диалога"],
        UiText::DataSourceDisabled => ["已在当前会话停用", "已在此對話停用", "Disabled for this conversation", "Für diese Unterhaltung deaktiviert", "Desactivada para esta conversación", "Désactivée pour cette conversation", "この会話で無効にしました", "이 대화에서 사용 안 함", "Отключено для этого диалога"],
        UiText::DataSourceInherited => ["已恢复默认策略", "已恢復預設原則", "Default policy restored", "Standardrichtlinie wiederhergestellt", "Política predeterminada restaurada", "Politique par défaut restaurée", "既定のポリシーに戻しました", "기본 정책 복원됨", "Политика по умолчанию восстановлена"],
        UiText::DataSourceNoSession => ["当前没有可用会话", "目前沒有可用對話", "No active conversation", "Keine aktive Unterhaltung", "No hay una conversación activa", "Aucune conversation active", "アクティブな会話がありません", "활성 대화 없음", "Нет активного диалога"],
        UiText::DataSourceEmpty => ["尚未添加数据源", "尚未新增資料來源", "No data sources yet", "Noch keine Datenquellen", "Aún no hay fuentes de datos", "Aucune source de données", "データソースはまだありません", "데이터 소스가 아직 없습니다", "Источники данных ещё не добавлены"],
        UiText::DataSourceUnknownAction => ["未知的数据源操作", "未知的資料來源操作", "Unknown data-source action", "Unbekannte Datenquellenaktion", "Acción de fuente de datos desconocida", "Action de source de données inconnue", "不明なデータソース操作です", "알 수 없는 데이터 소스 작업입니다", "Неизвестное действие с источником данных"],
        UiText::DataSourcePolicy => ["策略", "原則", "Policy", "Richtlinie", "Política", "Politique", "ポリシー", "정책", "Политика"],
        UiText::DataSourceRecords => ["条使用记录", "筆使用紀錄", "usage records", "Nutzungsprotokolle", "registros de uso", "enregistrements d’utilisation", "件の利用記録", "사용 기록", "записей использования"],
        UiText::DataSourceVerified => ["已验证", "已驗證", "Verified", "Verifiziert", "Verificada", "Vérifiée", "検証済み", "확인됨", "Проверено"],
        UiText::PromptCache => ["提示缓存", "提示快取", "Prompt cache", "Prompt-Cache", "Caché de instrucciones", "Cache d’invite", "プロンプトキャッシュ", "프롬프트 캐시", "Кэш промпта"],
        UiText::CacheRead => ["读取", "讀取", "read", "gelesen", "leída", "lecture", "読み取り", "읽기", "чтение"],
        UiText::CacheWrite => ["写入", "寫入", "write", "geschrieben", "escrita", "écriture", "書き込み", "쓰기", "запись"],
        UiText::CacheSections => ["片段变化", "區段變更", "section changes", "Abschnittsänderungen", "cambios de sección", "changements de section", "セクション変更", "섹션 변경", "изменения разделов"],
        UiText::CacheDisabled => ["已关闭", "已停用", "disabled", "deaktiviert", "deshabilitada", "désactivé", "無効", "비활성화됨", "отключено"],
        UiText::CacheInvalidIdentity => ["标识无效", "識別無效", "invalid identity", "ungültige Identität", "identidad no válida", "identité non valide", "識別情報が無効", "잘못된 식별자", "недопустимый идентификатор"],
        UiText::CacheFirstRequest => ["首次请求", "首次請求", "first request", "erste Anfrage", "primera solicitud", "première requête", "初回リクエスト", "첫 번째 요청", "первый запрос"],
        UiText::CacheHit => ["命中", "命中", "hit", "Treffer", "acierto", "succès", "ヒット", "적중", "попадание"],
        UiText::CacheHitRate => ["缓存命中率", "快取命中率", "Cache hit rate", "Cache-Trefferrate", "Aciertos de caché", "Taux de cache", "キャッシュ命中率", "캐시 적중률", "Попадания в кэш"],
        UiText::CacheStored => ["已写入", "已寫入", "stored", "gespeichert", "guardada", "enregistré", "保存済み", "저장됨", "сохранено"],
        UiText::CacheMiss => ["未命中", "未命中", "miss", "nicht getroffen", "fallo", "échec", "ミス", "미적중", "промах"],
        UiText::CacheStableCandidate => ["稳定候选", "穩定候選", "stable candidate", "stabiler Kandidat", "candidata estable", "candidat stable", "安定候補", "안정 후보", "стабильный кандидат"],
        UiText::CacheUnknown => ["状态未知", "狀態未知", "unknown", "unbekannt", "desconocido", "inconnu", "不明", "알 수 없음", "неизвестно"],
        UiText::Sources => ["来源", "來源", "Sources", "Quellen", "Fuentes", "Sources", "出典", "출처", "Источники"],
        UiText::Page => ["页", "頁", "page", "Seite", "página", "page", "ページ", "페이지", "страница"],
        UiText::Lines => ["行", "行", "lines", "Zeilen", "líneas", "lignes", "行", "줄", "строки"],
        UiText::Answer => ["回答", "回答", "Answer", "Antwort", "Respuesta", "Réponse", "回答", "답변", "Ответ"],
        UiText::Session => ["会话", "工作階段", "Session", "Sitzung", "Sesión", "Session", "セッション", "세션", "Сеанс"],
        UiText::SessionBusy => ["正在执行", "執行中", "Running", "Wird ausgeführt", "En ejecución", "En cours", "実行中", "실행 중", "Выполняется"],
        UiText::WaitInOriginalTerminal => ["请在原终端中断或等待完成", "請在原終端機中斷或等待完成", "Wait or interrupt it in the original terminal", "Im ursprünglichen Terminal warten oder abbrechen", "Espera o interrumpe en el terminal original", "Attendez ou interrompez dans le terminal d’origine", "元のターミナルで待機または中断してください", "원래 터미널에서 대기하거나 중단하세요", "Дождитесь завершения или прервите в исходном терминале"],
        UiText::Plan => ["计划", "計畫", "Plan", "Plan", "Plan", "Plan", "計画", "계획", "План"],
        UiText::AllComplete => ["全部完成", "全部完成", "All complete", "Alles abgeschlossen", "Todo completado", "Tout est terminé", "すべて完了", "모두 완료", "Всё завершено"],
        UiText::WaitingNextStep => ["等待下一步", "等待下一步", "Waiting for next step", "Warten auf den nächsten Schritt", "Esperando el siguiente paso", "En attente de l’étape suivante", "次のステップを待機中", "다음 단계 대기 중", "Ожидание следующего шага"],
        UiText::Background => ["后台", "背景", "Background", "Hintergrund", "Segundo plano", "Arrière-plan", "バックグラウンド", "백그라운드", "Фон"],
        UiText::TasksUnit => ["个任务", "項任務", "tasks", "Aufgaben", "tareas", "tâches", "件のタスク", "개 작업", "задач"],
        UiText::RunningUnit => ["个运行中", "項執行中", "running", "aktiv", "en ejecución", "en cours", "件を実行中", "개 실행 중", "выполняется"],
        UiText::ManageJobs => ["用 /jobs 管理", "以 /jobs 管理", "manage with /jobs", "mit /jobs verwalten", "gestionar con /jobs", "gérer avec /jobs", "/jobs で管理", "/jobs 로 관리", "управление через /jobs"],
        UiText::MoreTasks => ["其余任务请用 /jobs 查看", "其餘任務請用 /jobs 查看", "more tasks; use /jobs", "weitere Aufgaben; /jobs verwenden", "tareas más; usa /jobs", "tâches supplémentaires ; utilisez /jobs", "件の残りは /jobs で確認", "개 남음 · /jobs 로 확인", "других задач; используйте /jobs"],
        UiText::Agents => ["子代理", "子代理", "Agents", "Agenten", "Agentes", "Agents", "サブエージェント", "하위 에이전트", "Агенты"],
        UiText::AgentUsage => ["用法：/agent <类型> <任务>", "用法：/agent <類型> <任務>", "Usage: /agent <type> <task>", "Verwendung: /agent <Typ> <Aufgabe>", "Uso: /agent <tipo> <tarea>", "Utilisation : /agent <type> <tâche>", "使い方: /agent <種類> <タスク>", "사용법: /agent <유형> <작업>", "Использование: /agent <тип> <задача>"],
        UiText::Tasks => ["后台任务", "背景任務", "Tasks", "Aufgaben", "Tareas", "Tâches", "バックグラウンドタスク", "백그라운드 작업", "Задачи"],
        UiText::TasksUsage => ["用法：/tasks [<任务ID> | stop <任务ID>]", "用法：/tasks [<任務 ID> | stop <任務 ID>]", "Usage: /tasks [<task-id> | stop <task-id>]", "Verwendung: /tasks [<Aufgaben-ID> | stop <Aufgaben-ID>]", "Uso: /tasks [<id-tarea> | stop <id-tarea>]", "Utilisation : /tasks [<id-tâche> | stop <id-tâche>]", "使い方: /tasks [<タスクID> | stop <タスクID>]", "사용법: /tasks [<작업-ID> | stop <작업-ID>]", "Использование: /tasks [<ID-задачи> | stop <ID-задачи>]"],
        UiText::ResumeUsage => ["用法：/resume", "用法：/resume", "Usage: /resume", "Verwendung: /resume", "Uso: /resume", "Utilisation : /resume", "使い方: /resume", "사용법: /resume", "Использование: /resume"],
        UiText::ContinueUsage => ["用法：/continue", "用法：/continue", "Usage: /continue", "Verwendung: /continue", "Uso: /continue", "Utilisation : /continue", "使い方: /continue", "사용법: /continue", "Использование: /continue"],
        UiText::AutonomousGoal => ["自主目标", "自主目標", "Autonomous goal", "Autonomes Ziel", "Objetivo autónomo", "Objectif autonome", "自律目標", "자율 목표", "Автономная цель"],
        UiText::Constraints => ["约束", "限制條件", "Constraints", "Einschränkungen", "Restricciones", "Contraintes", "制約", "제약 조건", "Ограничения"],
        UiText::DefaultSafeConstraints => ["使用默认安全约束", "使用預設安全限制", "Using default safety constraints", "Standard-Sicherheitsregeln werden verwendet", "Se usan las restricciones de seguridad predeterminadas", "Contraintes de sécurité par défaut utilisées", "既定の安全制約を使用", "기본 안전 제약 조건 사용", "Используются стандартные ограничения безопасности"],
        UiText::Mode => ["模式", "模式", "Mode", "Modus", "Modo", "Mode", "モード", "모드", "Режим"],
        UiText::AutonomousUntilComplete => ["持续推进，完成后自动报告", "持續推進，完成後自動回報", "Continue autonomously until complete", "Autonom bis zum Abschluss fortfahren", "Continuar de forma autónoma hasta terminar", "Continuer de façon autonome jusqu’à la fin", "完了まで自律的に継続", "완료될 때까지 자율적으로 계속", "Продолжать автономно до завершения"],
        UiText::NoAvailableModels => ["没有可用模型", "沒有可用模型", "No available models", "Keine Modelle verfügbar", "No hay modelos disponibles", "Aucun modèle disponible", "利用可能なモデルがありません", "사용 가능한 모델이 없습니다", "Нет доступных моделей"],
        UiText::HelpAgent => ["启动或列出可用的子代理", "啟動或列出可用的子代理", "Start or list available sub-agents", "Unteragenten starten oder auflisten", "Iniciar o listar subagentes", "Démarrer ou lister les sous-agents", "サブエージェントを開始または一覧表示", "사용 가능한 하위 에이전트 시작 또는 목록 보기", "Запустить или показать субагентов"],
        UiText::Work => ["工作", "工作", "Work", "Arbeit", "Trabajo", "Travail", "作業", "작업", "Работа"],
        UiText::WorkUsage => ["用法：/work [<工作ID> | status|stop|pause|resume|rerun <工作ID>]", "用法：/work [<工作 ID> | status|stop|pause|resume|rerun <工作 ID>]", "Usage: /work [<work-id> | status|stop|pause|resume|rerun <work-id>]", "Verwendung: /work [<Arbeits-ID> | status|stop|pause|resume|rerun <Arbeits-ID>]", "Uso: /work [<id-trabajo> | status|stop|pause|resume|rerun <id-trabajo>]", "Utilisation : /work [<id-travail> | status|stop|pause|resume|rerun <id-travail>]", "使い方: /work [<作業ID> | status|stop|pause|resume|rerun <作業ID>]", "사용법: /work [<작업-ID> | status|stop|pause|resume|rerun <작업-ID>]", "Использование: /work [<ID-работы> | status|stop|pause|resume|rerun <ID-работы>]"],
        UiText::WorkResolveHint => ["外部结果仍未知。保留原记录；只能通过精确执行恢复控制核验回执、授权风险重试或保持受阻。", "外部結果仍未知。請保留原始紀錄；僅能透過精確執行復原控制核驗回執、授權風險重試或維持受阻。", "The external outcome is unknown. Preserve the record; use exact execution recovery to verify a receipt, authorize a risky retry, or remain blocked.", "Das externe Ergebnis ist unbekannt. Den Datensatz behalten; eine Wiederherstellung erfordert den genauen Ausführungsnachweis oder eine ausdrückliche Risikoentscheidung.", "El resultado externo es desconocido. Conserva el registro; la recuperación exige el recibo exacto o una decisión explícita sobre el riesgo.", "Le résultat externe est inconnu. Conservez la trace ; la reprise exige un reçu exact ou une décision explicite sur le risque.", "外部結果は不明です。記録を保持し、正確な実行の復旧操作で受領証または明示的なリスク判断を確認してください。", "외부 결과를 알 수 없습니다. 기록을 유지하고 정확한 실행 복구 제어로 영수증을 확인하거나 위험한 재시도를 승인하거나 차단 상태를 유지하세요.", "Внешний результат неизвестен. Сохраните запись; восстановление требует точной квитанции или явного решения о риске."],
        UiText::HelpRoom => [
            "发送房间消息。输入 @ 后按 Tab 选择成员，省略 @ 由协调者分派。",
            "傳送房間訊息。輸入 @ 後按 Tab 選擇成員，省略 @ 由協調者分派。",
            "Send a room message. Type @ and press Tab to choose a member, or omit @ for automatic routing.",
            "Nachricht an den Raum senden. Mit @ und Tab ein Mitglied wählen, ohne @ automatisch zuweisen.",
            "Envía un mensaje a la sala. Escribe @ y pulsa Tab para elegir un miembro, u omite @ para la asignación automática.",
            "Envoyer un message au salon. Saisir @ puis Tab pour choisir un membre, ou omettre @ pour une attribution automatique.",
            "ルームに送信します。@ と Tab でメンバーを選び、省略すると自動で割り当てます。",
            "방에 메시지를 보냅니다. @ 입력 후 Tab으로 멤버를 선택하거나 @를 생략해 자동으로 배정합니다.",
            "Отправить сообщение в комнату. Введите @ и нажмите Tab для выбора участника или опустите @ для автоназначения.",
        ],
        UiText::DeclineQuestion => [
            "拒绝回答", "拒絕回答", "Decline to answer", "Antwort ablehnen", "Rechazar la pregunta",
            "Refuser de répondre", "回答を辞退", "답변 거절", "Отказаться от ответа",
        ],
        UiText::QuestionDeferred => [
            "问题已保留，输入 /questions 继续回答。", "已保留問題，輸入 /questions 繼續回答。",
            "Question saved. Use /questions to answer it.", "Frage zurückgestellt. Mit /questions antworten.",
            "Pregunta pendiente. Usa /questions para responder.", "Question en attente. Tapez /questions pour répondre.",
            "質問は保留中です。/questions で回答できます。", "질문을 보류했습니다. /questions 로 답변하세요.",
            "Вопрос отложен. Введите /questions, чтобы ответить.",
        ],
        UiText::QuestionKeyHint => [
            "↑↓ 选择 · Enter 确认 · Esc 稍后回答", "↑↓ 選擇 · Enter 確認 · Esc 稍後回答",
            "↑↓ select · Enter to confirm · Esc to answer later", "↑↓ wählen · Enter bestätigen · Esc später antworten",
            "↑↓ elegir · Intro confirmar · Esc responder después", "↑↓ choisir · Entrée confirmer · Échap répondre plus tard",
            "↑↓ 選択 · Enter 確定 · Esc 後で回答", "↑↓ 선택 · Enter 확인 · Esc 나중에 답변",
            "↑↓ выбор · Enter подтвердить · Esc ответить позже",
        ],
        UiText::QuestionLineHint => [
            "/decline 拒绝回答 · q 稍后回答", "/decline 拒絕回答 · q 稍後回答", "/decline to decline · q to answer later",
            "/decline ablehnen · q später antworten", "/decline rechazar · q responder después", "/decline refuser · q répondre plus tard",
            "/decline 辞退 · q 後で回答", "/decline 거절 · q 나중에 답변", "/decline отказаться · q ответить позже",
        ],
        UiText::NoPendingQuestions => [
            "当前没有待回答的问题。", "目前沒有待回答的問題。", "No pending questions.",
            "Keine offenen Fragen.", "No hay preguntas pendientes.", "Aucune question en attente.",
            "回答待ちの質問はありません。", "답변을 기다리는 질문이 없습니다.", "Нет вопросов, ожидающих ответа.",
        ],
        UiText::NoRooms => [
            "还没有房间。请在终端创建一个房间", "還沒有房間。請在終端機建立房間",
            "No rooms yet. Create one in your terminal", "Noch keine Räume. Im Terminal einen Raum erstellen",
            "Todavía no hay salas. Crea una en el terminal", "Aucun salon. Créez-en un dans le terminal",
            "ルームがありません。ターミナルで作成してください", "방이 없습니다. 터미널에서 방을 만드세요",
            "Комнат пока нет. Создайте комнату в терминале",
        ],
        UiText::PermissionInstructionOption => [
            "I. 补充说明，操作将继续等待授权", "I. 補充說明，操作將繼續等待授權",
            "I. Add instructions and keep the action paused", "I. Hinweis ergänzen und Freigabe abwarten",
            "I. Añadir instrucciones y dejar la acción en pausa", "I. Ajouter une consigne et attendre l’autorisation",
            "I. 指示を追加して許可を待つ", "I. 지침을 추가하고 승인 대기", "I. Добавить указание и ждать разрешения",
        ],
        UiText::PermissionInstructionCommand => [
            "输入 i 和说明文字后按 Enter 保存", "輸入 i 和說明文字後按 Enter 儲存",
            "Type i followed by your instructions and press Enter", "i und den Hinweis eingeben, dann Enter drücken",
            "Escribe i y las instrucciones, y pulsa Enter", "Saisissez i puis votre consigne et appuyez sur Entrée",
            "i に続けて指示を入力し、Enter で保存", "i 다음에 지침을 입력하고 Enter로 저장", "Введите i, текст указания и нажмите Enter",
        ],
        UiText::NoConfiguredMcp => [
            "尚未配置 MCP 服务。运行 nola web，在「技能与 MCP」中添加。", "尚未設定 MCP 服務。執行 nola web，在「技能與 MCP」中新增。",
            "No MCP servers configured. Run nola web and add one under Skills & MCP.",
            "Keine MCP-Server eingerichtet. Mit nola web unter Skills & MCP einen Server hinzufügen.",
            "No hay servidores MCP configurados. Ejecuta nola web y añade uno en Skills y MCP.",
            "Aucun serveur MCP configuré. Lancez nola web et ajoutez-en un dans Skills et MCP.",
            "MCP サーバーは未設定です。nola web のスキルと MCP から追加できます。",
            "설정된 MCP 서버가 없습니다. nola web을 실행하고 스킬 및 MCP에서 추가하세요.",
            "Серверов MCP пока нет. Запустите nola web и добавьте сервер в разделе навыков и MCP.",
        ],
        UiText::NoAvailableSkills => ["尚无可用技能。", "尚無可用技能。", "No skills available.", "Keine Skills verfügbar.", "No hay habilidades disponibles.", "Aucune compétence disponible.", "利用できるスキルはありません。", "사용 가능한 스킬이 없습니다.", "Нет доступных навыков."],
        UiText::MemoryHeading => ["长期记忆", "長期記憶", "Memory", "Erinnerungen", "Memoria", "Mémoire", "メモリ", "메모리", "Память"],
        UiText::MemorySearchHeading => ["搜索记忆", "搜尋記憶", "Memory search", "Erinnerungen suchen", "Buscar recuerdos", "Rechercher dans la mémoire", "メモリ検索", "메모리 검색", "Поиск в памяти"],
        UiText::MemorySaved => ["已保存记忆", "已儲存記憶", "Saved memory", "Erinnerung gespeichert", "Recuerdo guardado", "Souvenir enregistré", "メモリを保存しました", "메모리를 저장했습니다", "Запись сохранена"],
        UiText::NoSavedMemory => ["尚无可见的记忆。", "尚無可見的記憶。", "No visible memories yet.", "Noch keine sichtbaren Erinnerungen.", "Aún no hay recuerdos visibles.", "Aucun souvenir visible.", "表示できるメモリはありません。", "표시할 메모리가 없습니다.", "Пока нет видимых записей памяти."],
        UiText::NoMatchingMemory => ["没有找到匹配的记忆。", "沒有找到符合的記憶。", "No matching memories.", "Keine passenden Erinnerungen.", "No se encontraron recuerdos.", "Aucun souvenir correspondant.", "一致するメモリはありません。", "일치하는 메모리가 없습니다.", "Совпадений в памяти не найдено."],
        UiText::HelpWork => ["统一查看和控制目标、任务、后台进程与外部执行器", "統一檢視與控制目標、任務、背景處理程序與外部執行器", "Inspect and control goals, tasks, background processes, and external executors", "Ziele, Aufgaben, Hintergrundprozesse und externe Ausführer gemeinsam verwalten", "Inspeccionar y controlar objetivos, tareas, procesos en segundo plano y ejecutores externos", "Afficher et contrôler les objectifs, tâches, processus en arrière-plan et exécuteurs externes", "目標、タスク、バックグラウンド処理、外部実行環境をまとめて管理", "목표, 작업, 백그라운드 프로세스 및 외부 실행기 검사 및 제어", "Просмотр и управление целями, задачами, фоновыми процессами и внешними исполнителями"],
        UiText::NoWork => ["当前会话没有工作记录。", "目前工作階段沒有工作紀錄。", "No work in this conversation.", "Keine Arbeit in dieser Unterhaltung.", "No hay trabajo en esta conversación.", "Aucun travail dans cette conversation.", "この会話に作業はありません。", "이 대화에 작업 기록이 없습니다.", "В этом диалоге нет работы."],
        UiText::WorkActive => ["项进行中", "項進行中", "active", "aktiv", "activos", "actifs", "件進行中", "개 활성", "активно"],
        UiText::WorkFooter => ["使用 `/work <工作ID>` 查看详情；使用 `/work stop <工作ID>` 停止。已结束的流程可用 `/work rerun <工作ID>` 按当前定义重新运行。", "使用 `/work <工作 ID>` 檢視詳細資料；使用 `/work stop <工作 ID>` 停止。已結束的流程可用 `/work rerun <工作 ID>` 依最新定義重新執行。", "Use `/work <work-id>` for details and `/work stop <work-id>` to stop. A finished SOP can be started as a new run with `/work rerun <work-id>`.", "Details mit `/work <Arbeits-ID>`, stoppen mit `/work stop <Arbeits-ID>`. Einen beendeten SOP-Ablauf starten Sie mit `/work rerun <Arbeits-ID>` als neuen Lauf.", "Usa `/work <id-trabajo>` para ver detalles y `/work stop <id-trabajo>` para detener. Un SOP finalizado puede iniciarse como una ejecución nueva con `/work rerun <id-trabajo>`.", "Utilisez `/work <id-travail>` pour les détails et `/work stop <id-travail>` pour arrêter. Un SOP terminé peut être relancé dans une nouvelle exécution avec `/work rerun <id-travail>`.", "`/work <作業ID>` で詳細、`/work stop <作業ID>` で停止します。終了した SOP は `/work rerun <作業ID>` で新しい実行として開始できます。", "세부 정보는 `/work <작업-ID>`, 중지는 `/work stop <작업-ID>`를 사용하세요. 완료된 SOP는 `/work rerun <작업-ID>`로 새 실행을 시작할 수 있습니다.", "Используйте `/work <ID-работы>` для подробностей и `/work stop <ID-работы>` для остановки. Завершённый SOP можно запустить заново командой `/work rerun <ID-работы>`."],
        UiText::WorkId => ["工作 ID", "工作 ID", "Work ID", "Arbeits-ID", "ID de trabajo", "ID du travail", "作業 ID", "작업 ID", "ID работы"],
        UiText::WorkKind => ["类型", "類型", "Kind", "Typ", "Tipo", "Type", "種類", "유형", "Тип"],
        UiText::WorkParent => ["上级工作", "上層工作", "Parent work", "Übergeordnete Arbeit", "Trabajo principal", "Travail parent", "親作業", "상위 작업", "Родительская работа"],
        UiText::WorkCurrentStep => ["当前步骤", "目前步驟", "Current step", "Aktueller Schritt", "Paso actual", "Étape actuelle", "現在のステップ", "현재 단계", "Текущий шаг"],
        UiText::WorkSopVersion => ["流程版本", "流程版本", "SOP version", "SOP-Version", "Versión del SOP", "Version du SOP", "SOP バージョン", "SOP 버전", "Версия SOP"],
        UiText::WorkSopUpdateAvailable => ["有新的流程定义", "有新的流程定義", "Definition update available", "Neue SOP-Definition verfügbar", "Hay una definición nueva disponible", "Nouvelle définition disponible", "新しい定義があります", "정의 업데이트 사용 가능", "Доступно обновление определения"],
        UiText::WorkSopPinnedHint => ["当前运行继续使用已固定的定义；结束后可运行 `/work rerun <工作ID>`，以最新定义创建新运行。", "目前執行仍固定於原始定義；結束後可執行 `/work rerun <工作 ID>` 依最新定義建立新執行。", "This run remains pinned to its original definition. After it ends, use `/work rerun <work-id>` to create a new run from the latest definition.", "Dieser Lauf bleibt an seine ursprüngliche Definition gebunden. Danach erstellt `/work rerun <Arbeits-ID>` einen neuen Lauf mit der neuesten Definition.", "Esta ejecución permanece fijada a su definición original. Cuando termine, usa `/work rerun <id-trabajo>` para crear otra con la definición más reciente.", "Cette exécution reste liée à sa définition d’origine. Une fois terminée, utilisez `/work rerun <id-travail>` pour créer une nouvelle exécution avec la dernière définition.", "この実行は開始時の定義に固定されています。終了後に `/work rerun <作業ID>` を使うと最新定義で新しい実行を作成できます。", "이 실행은 원래 정의에 고정되어 있습니다. 종료 후 `/work rerun <작업-ID>`로 최신 정의에서 새 실행을 만들 수 있습니다.", "Этот запуск остаётся привязан к исходному определению. После завершения используйте `/work rerun <ID-работы>`, чтобы создать новый запуск с актуальным определением."],
        UiText::WorkRerunStarted => ["已创建新的流程运行", "已建立新的流程執行", "Started a new SOP run", "Neuen SOP-Lauf gestartet", "Se inició una ejecución SOP nueva", "Nouvelle exécution SOP créée", "新しい SOP 実行を開始しました", "새 SOP 실행을 시작했습니다", "Создан новый запуск SOP"],
        UiText::WorkRerunRequiresTerminal => ["请先停止或完成原流程运行，再创建新运行。", "請先停止或完成原流程執行，再建立新執行。", "Stop or finish the original SOP run before creating a new run.", "Beenden oder stoppen Sie den ursprünglichen SOP-Lauf, bevor Sie einen neuen erstellen.", "Detén o finaliza la ejecución SOP original antes de crear otra.", "Arrêtez ou terminez l’exécution SOP d’origine avant d’en créer une nouvelle.", "新しい実行を作成する前に、元の SOP 実行を停止または完了してください。", "새 실행을 만들기 전에 원래 SOP 실행을 중지하거나 완료하세요.", "Перед созданием нового запуска остановите или завершите исходный запуск SOP."],
        UiText::WorkSopDefinitionUnavailable => ["当前客户端没有可用于该流程的新定义。", "目前用戶端沒有可用於此流程的新定義。", "No current definition is available for this SOP.", "Für diesen SOP ist keine aktuelle Definition verfügbar.", "No hay una definición actual disponible para este SOP.", "Aucune définition actuelle n’est disponible pour ce SOP.", "この SOP に利用できる現在の定義がありません。", "이 SOP에 사용할 수 있는 최신 정의가 없습니다.", "Для этого SOP нет доступного актуального определения."],
        UiText::WorkNotFound => ["未找到工作", "找不到工作", "Work not found", "Arbeit nicht gefunden", "Trabajo no encontrado", "Travail introuvable", "作業が見つかりません", "작업을 찾을 수 없습니다", "Работа не найдена"],
        UiText::WorkControlUnsupported => ["该工作类型暂不支持此操作。", "此工作類型暫不支援該操作。", "This control is not supported for that work type.", "Diese Aktion wird für diesen Arbeitstyp nicht unterstützt.", "Este control no es compatible con ese tipo de trabajo.", "Cette commande n’est pas prise en charge pour ce type de travail.", "この作業タイプではその操作を利用できません。", "이 작업 유형에서는 해당 제어가 지원되지 않습니다.", "Эта операция не поддерживается для данного типа работы."],
        UiText::WorkStopped => ["已请求停止", "已要求停止", "Stop requested", "Stopp angefordert", "Detención solicitada", "Arrêt demandé", "停止を要求しました", "중지 요청됨", "Запрошена остановка"],
        UiText::WorkPaused => ["已暂停", "已暫停", "Paused", "Pausiert", "En pausa", "En pause", "一時停止しました", "일시 중지됨", "Приостановлено"],
        UiText::WorkResumed => ["已继续", "已繼續", "Resumed", "Fortgesetzt", "Reanudado", "Repris", "再開しました", "재개됨", "Возобновлено"],
        UiText::WorkGoal => ["目标", "目標", "Goal", "Ziel", "Objetivo", "Objectif", "目標", "목표", "Цель"],
        UiText::WorkSop => ["标准流程", "標準流程", "SOP run", "SOP-Ablauf", "Ejecución SOP", "Exécution SOP", "SOP 実行", "SOP 실행", "Выполнение SOP"],
        UiText::WorkSkill => ["技能运行", "技能執行", "Skill run", "Skill-Ausführung", "Ejecución de habilidad", "Exécution de compétence", "スキル実行", "스킬 실행", "Запуск навыка"],
        UiText::WorkTask => ["任务", "任務", "Task", "Aufgabe", "Tarea", "Tâche", "タスク", "작업", "Задача"],
        UiText::WorkProcess => ["后台进程", "背景處理程序", "Background process", "Hintergrundprozess", "Proceso en segundo plano", "Processus en arrière-plan", "バックグラウンド処理", "백그라운드 프로세스", "Фоновый процесс"],
        UiText::WorkExecutor => ["外部执行器", "外部執行器", "External executor", "Externer Ausführer", "Ejecutor externo", "Exécuteur externe", "外部実行環境", "외부 실행기", "Внешний исполнитель"],
        UiText::WorkQueued => ["排队中", "排隊中", "Queued", "In Warteschlange", "En cola", "En file d’attente", "待機中", "대기 중", "В очереди"],
        UiText::InputQueueFull => ["队列已满，输入已保留", "佇列已滿，已保留輸入", "Queue full; draft retained", "Warteschlange voll; Entwurf behalten", "Cola llena; borrador conservado", "File pleine ; brouillon conservé", "待機列がいっぱいです。下書きは保持されました", "대기열이 가득 찼습니다. 초안은 유지됩니다", "Очередь заполнена; черновик сохранён"],
        UiText::InputAdmissionFailed => ["未确认输入已接收，草稿已保留", "未確認已接收輸入，已保留草稿", "Input admission unconfirmed; draft retained", "Eingangsbestätigung fehlt; Entwurf behalten", "Recepción sin confirmar; borrador conservado", "Réception non confirmée ; brouillon conservé", "入力の受領を確認できません。下書きは保持されました", "입력 수락이 확인되지 않았습니다. 초안은 유지됩니다", "Приём ввода не подтверждён; черновик сохранён"],
        UiText::RecoveryBlockedUseResume => [
            "恢复尚未完成，未启动新任务。请用 /resume 重试或选择其他会话。",
            "復原尚未完成，未啟動新任務。請用 /resume 重試或選擇其他工作階段。",
            "Recovery has not completed; no new task was started. Use /resume to retry or switch to another session.",
            "Die Wiederherstellung wurde noch nicht abgeschlossen; es wurde keine neue Aufgabe gestartet. Nutzen Sie /resume oder wechseln Sie die Sitzung.",
            "La recuperación no se ha completado; no se inició una tarea nueva. Usa /resume para reintentar o cambia de sesión.",
            "La reprise n’est pas terminée ; aucune nouvelle tâche n’a été lancée. Utilisez /resume pour réessayer ou changez de session.",
            "復旧が完了していないため、新しいタスクを開始しませんでした。/resume で再試行するか別のセッションを選択してください。",
            "복구가 완료되지 않아 새 작업을 시작하지 않았습니다. /resume 로 다시 시도하거나 다른 세션을 선택하세요.",
            "Восстановление ещё не завершено; новая задача не запущена. Используйте /resume для повтора или выберите другой сеанс.",
        ],
        UiText::WorkPlanning => ["规划中", "規劃中", "Planning", "Planung", "Planificando", "Planification", "計画中", "계획 중", "Планирование"],
        UiText::WorkReady => ["就绪", "就緒", "Ready", "Bereit", "Listo", "Prêt", "準備完了", "준비됨", "Готово"],
        UiText::WorkWaitingUser => ["等待用户", "等待使用者", "Waiting for user", "Wartet auf Benutzer", "Esperando al usuario", "En attente de l’utilisateur", "ユーザー待ち", "사용자 대기 중", "Ожидание пользователя"],
        UiText::WorkWaitingExternal => ["等待外部操作", "等待外部操作", "Waiting for external work", "Wartet auf externe Arbeit", "Esperando trabajo externo", "En attente d’un travail externe", "外部処理待ち", "외부 작업 대기 중", "Ожидание внешней работы"],
        UiText::WorkVerifying => ["验证中", "驗證中", "Verifying", "Überprüfung", "Verificando", "Vérification", "検証中", "검증 중", "Проверка"],
        UiText::WorkCompensating => ["补偿中", "補償中", "Compensating", "Kompensation", "Compensando", "Compensation", "補償中", "보상 중", "Компенсация"],
        UiText::WorkArtifacts => ["产物", "產物", "Artifacts", "Artefakte", "Artefactos", "Artefacts", "成果物", "아티팩트", "Артефакты"],
        UiText::WorkEvidence => ["证据", "證據", "Evidence", "Nachweise", "Evidencia", "Preuves", "証拠", "증거", "Доказательства"],
        UiText::HelpTasks => ["查看或停止后台任务；权限请求会自动弹出", "檢視或停止背景任務；權限請求會自動顯示", "Inspect or stop background tasks; permission prompts open automatically", "Hintergrundaufgaben anzeigen oder stoppen; Berechtigungen öffnen automatisch", "Ver o detener tareas en segundo plano; los permisos se abren automáticamente", "Voir ou arrêter les tâches en arrière-plan ; les autorisations s’ouvrent automatiquement", "バックグラウンドタスクの確認・停止。権限画面は自動表示", "백그라운드 작업 확인 또는 중지; 권한 요청이 자동으로 열립니다", "Просмотреть или остановить фоновые задачи; запрос разрешения откроется автоматически"],
        UiText::HelpJobs => ["管理后台任务", "管理背景任務", "Manage background tasks", "Hintergrundaufgaben verwalten", "Gestionar tareas en segundo plano", "Gérer les tâches en arrière-plan", "バックグラウンドタスクを管理", "백그라운드 작업 관리", "Управление фоновыми задачами"],
        UiText::GoalPaused => ["目标已暂停", "目標已暫停", "Goal paused", "Ziel pausiert", "Objetivo en pausa", "Objectif en pause", "目標を一時停止", "목표 일시 중지됨", "Цель приостановлена"],
        UiText::ConfigNotFound => ["未找到 Nola 配置：", "找不到 Nola 設定：", "Nola config was not found at", "Nola-Konfiguration nicht gefunden:", "No se encontró la configuración de Nola en", "Configuration Nola introuvable :", "Nola 設定が見つかりません:", "Nola 설정을 찾을 수 없습니다:", "Конфигурация Nola не найдена:"],
        UiText::CreateConfigPrompt => ["现在创建？[Y/n] ", "現在建立？[Y/n] ", "Create it now? [Y/n] ", "Jetzt erstellen? [Y/n] ", "¿Crearla ahora? [Y/n] ", "La créer maintenant ? [Y/n] ", "今すぐ作成しますか？ [Y/n] ", "지금 만드시겠습니까? [Y/n] ", "Создать сейчас? [Y/n] "],
        UiText::EndpointPrompt => ["LLM 端点（通常以 /v1 结尾）：", "LLM 端點（通常以 /v1 結尾）：", "LLM endpoint, usually ending with /v1: ", "LLM-Endpunkt, meist mit /v1 am Ende: ", "Endpoint LLM, normalmente termina en /v1: ", "Endpoint LLM, se terminant généralement par /v1 : ", "LLM エンドポイント（通常 /v1 で終わります）: ", "LLM 엔드포인트 (보통 /v1로 끝남): ", "Эндпоинт LLM, обычно с /v1 в конце: "],
        UiText::EndpointRequired => ["端点不能为空。", "端點不能為空。", "Endpoint is required.", "Endpunkt ist erforderlich.", "El endpoint es obligatorio.", "L’endpoint est requis.", "エンドポイントは必須です。", "엔드포인트는 필수입니다.", "Эндпоинт обязателен."],
        UiText::EndpointInvalidScheme => ["端点必须以 http:// 或 https:// 开头。", "端點必須以 http:// 或 https:// 開頭。", "Endpoint must start with http:// or https://.", "Der Endpunkt muss mit http:// oder https:// beginnen.", "El endpoint debe comenzar con http:// o https://.", "L’endpoint doit commencer par http:// ou https://.", "エンドポイントは http:// または https:// で始まる必要があります。", "엔드포인트는 http:// 또는 https://로 시작해야 합니다.", "Эндпоинт должен начинаться с http:// или https://."],
        UiText::ModelPrompt => ["模型名称（例如 gpt-4o）：", "模型名稱（例如 gpt-4o）：", "Model ID (e.g. gpt-4o): ", "Modell-ID (z. B. gpt-4o): ", "ID del modelo (p. ej. gpt-4o): ", "ID du modèle (ex. gpt-4o) : ", "モデル ID（例: gpt-4o）: ", "모델 ID (예: gpt-4o): ", "ID модели (напр. gpt-4o): "],
        UiText::ModelRequired => ["模型名称不能为空。", "模型名稱不能為空。", "Model ID is required.", "Modell-ID ist erforderlich.", "El ID del modelo es obligatorio.", "L’ID du modèle est requis.", "モデル ID は必須です。", "모델 ID는 필수입니다.", "ID модели обязателен."],
        UiText::ApiKeyPrompt => ["API Key（可选，本地模型可留空）：", "API Key（選填，本機模型可留空）：", "API key (optional, leave empty for local models): ", "API-Key (optional, für lokale Modelle leer lassen): ", "Clave API (opcional, vacía para modelos locales): ", "Clé API (facultative, vide pour les modèles locaux) : ", "API キー（任意、ローカルモデルは空欄可）: ", "API 키 (선택 사항, 로컬 모델은 비워 두기): ", "API-ключ (необязательно, для локальных моделей можно оставить пустым): "],
        UiText::ConfigCreated => ["已创建", "已建立", "Created", "Erstellt", "Creado", "Créé", "作成しました", "생성됨", "Создано"],
        UiText::InitCheckOutput => ["检查输出位置", "檢查輸出位置", "Check output location", "Ausgabeort prüfen", "Comprobar ubicación de salida", "Vérifier l’emplacement de sortie", "出力先を確認", "출력 위치 확인", "Проверка места вывода"],
        UiText::InitScanFiles => ["扫描项目文件", "掃描專案檔案", "Scan project files", "Projektdateien scannen", "Examinar archivos del proyecto", "Analyser les fichiers du projet", "プロジェクトファイルを走査", "프로젝트 파일 스캔", "Сканирование файлов проекта"],
        UiText::InitAnalyzeProject => ["分析项目结构", "分析專案結構", "Analyze project structure", "Projektstruktur analysieren", "Analizar estructura del proyecto", "Analyser la structure du projet", "プロジェクト構造を分析", "프로젝트 구조 분석", "Анализ структуры проекта"],
        UiText::InitGenerateMemory => ["生成项目记忆", "產生專案記憶", "Generate project memory", "Projektspeicher erzeugen", "Generar memoria del proyecto", "Générer la mémoire du projet", "プロジェクトメモリを生成", "프로젝트 메모리 생성", "Создание памяти проекта"],
        UiText::InitWriteNola => ["写入 NOLA.md", "寫入 NOLA.md", "Write NOLA.md", "NOLA.md schreiben", "Escribir NOLA.md", "Écrire NOLA.md", "NOLA.md を書き込み", "NOLA.md 쓰기", "Запись NOLA.md"],
        UiText::NoBackgroundTasks => ["当前工作区没有后台任务。", "目前工作區沒有背景任務。", "No background tasks in this workspace.", "In diesem Arbeitsbereich gibt es keine Hintergrundaufgaben.", "No hay tareas en segundo plano en este espacio de trabajo.", "Aucune tâche en arrière-plan dans cet espace de travail.", "このワークスペースにバックグラウンドタスクはありません。", "이 작업 공간에 백그라운드 작업이 없습니다.", "В этой рабочей области нет фоновых задач."],
        UiText::BackgroundTasksTitle => ["# 后台任务", "# 背景任務", "# Background tasks", "# Hintergrundaufgaben", "# Tareas en segundo plano", "# Tâches en arrière-plan", "# バックグラウンドタスク", "# 백그라운드 작업", "# Фоновые задачи"],
        UiText::TasksFooter => ["使用 `/tasks <任务ID>` 查看结果，`/tasks stop <任务ID>` 停止任务；权限请求会自动打开审批面板。", "使用 `/tasks <任務 ID>` 檢視結果，`/tasks stop <任務 ID>` 停止任務；權限請求會自動開啟核准面板。", "Use `/tasks <task-id>` to inspect results and `/tasks stop <task-id>` to stop a task; permission requests open the approval panel automatically.", "Mit `/tasks <Aufgaben-ID>` Ergebnisse anzeigen und mit `/tasks stop <Aufgaben-ID>` stoppen; Berechtigungsanfragen öffnen automatisch den Dialog.", "Usa `/tasks <id-tarea>` para ver resultados y `/tasks stop <id-tarea>` para detenerla; los permisos abren el panel automáticamente.", "Utilisez `/tasks <id-tâche>` pour voir les résultats et `/tasks stop <id-tâche>` pour l’arrêter ; les autorisations ouvrent automatiquement le panneau.", "`/tasks <タスクID>` で結果を確認し、`/tasks stop <タスクID>` で停止します。権限要求は自動的に承認画面を開きます。", "결과를 확인하려면 `/tasks <작업-ID>`, 중지하려면 `/tasks stop <작업-ID>`를 사용하세요. 권한 요청 시 승인 패널이 자동으로 열립니다.", "Используйте `/tasks <ID-задачи>` для результатов и `/tasks stop <ID-задачи>` для остановки; запросы разрешений автоматически откроют панель."],
        UiText::Task => ["任务", "任務", "Task", "Aufgabe", "Tarea", "Tâche", "タスク", "작업", "Задача"],
        UiText::Status => ["状态", "狀態", "Status", "Status", "Estado", "État", "状態", "상태", "Статус"],
        UiText::Agent => ["代理", "代理", "Agent", "Agent", "Agente", "Agent", "エージェント", "에이전트", "Агент"],
        UiText::Progress => ["进度", "進度", "Progress", "Fortschritt", "Progreso", "Progression", "進捗", "진행률", "Прогресс"],
        UiText::Result => ["结果", "結果", "Result", "Ergebnis", "Resultado", "Résultat", "結果", "결과", "Результат"],
        UiText::Error => ["错误", "錯誤", "Error", "Fehler", "Error", "Erreur", "エラー", "오류", "Ошибка"],
        UiText::RecentActivity => ["最近活动", "最近活動", "Recent activity", "Letzte Aktivität", "Actividad reciente", "Activité récente", "最近のアクティビティ", "최근 활동", "Недавняя активность"],
        UiText::Queued => ["排队中", "排隊中", "Queued", "In Warteschlange", "En cola", "En file d’attente", "待機中", "대기 중", "В очереди"],
        UiText::Running => ["运行中", "執行中", "Running", "Läuft", "En ejecución", "En cours", "実行中", "실행 중", "Выполняется"],
        UiText::WaitingPermission => ["等待授权", "等待授權", "Waiting for approval", "Wartet auf Freigabe", "Esperando aprobación", "En attente d’autorisation", "承認待ち", "승인 대기 중", "Ожидание разрешения"],
        UiText::Completed => ["已完成", "已完成", "Completed", "Abgeschlossen", "Completada", "Terminée", "完了", "완료됨", "Завершено"],
        UiText::Failed => ["失败", "失敗", "Failed", "Fehlgeschlagen", "Fallida", "Échec", "失敗", "실패", "Ошибка"],
        UiText::GoalIntro => ["把要完成的结果写在下方，例如：检查 CLI 交互，修复问题并运行测试。Nola 会持续推进；需要你补充信息或授权时会暂停。", "請在下方填寫要達成的結果，例如：檢查 CLI 互動、修正問題並執行測試。Nola 會持續推進；需要您補充資訊或授權時會暫停。", "Describe the result below, for example: review the CLI, fix interaction issues, and run tests. Nola keeps working and pauses when it needs your input or permission.", "Beschreibe unten das Ergebnis. Nola arbeitet weiter und pausiert bei Rückfragen oder nötiger Erlaubnis.", "Describe abajo el resultado. Nola continúa y se detiene cuando necesita información o permiso.", "Décrivez le résultat ci-dessous. Nola continue et s’arrête si une précision ou une autorisation est nécessaire.", "下に達成したい結果を入力してください。Nola は作業を進め、確認や権限が必要な場合に一時停止します。", "아래에 달성할 결과를 입력하세요(예: CLI 상호작용 검토, 문제 수정 및 테스트 실행). Nola가 계속 진행하며 입력이나 권한이 필요할 때 일시 중지합니다.", "Опишите результат ниже. Nola продолжит работу и остановится, если нужны уточнения или разрешение."],
        UiText::SlashMenuKeys => ["↑↓ 选择 · Tab 补全 · Enter 执行 · Esc 收起", "↑↓ 選擇 · Tab 補全 · Enter 執行 · Esc 收起", "↑↓ select · Tab complete · Enter run · Esc close", "↑↓ wählen · Tab ergänzen · Enter ausführen · Esc schließen", "↑↓ elegir · Tab completar · Intro ejecutar · Esc cerrar", "↑↓ choisir · Tab compléter · Entrée exécuter · Échap fermer", "↑↓ 選択 · Tab 補完 · Enter 実行 · Esc 閉じる", "↑↓ 선택 · Tab 완성 · Enter 실행 · Esc 닫기", "↑↓ выбор · Tab дополнить · Enter выполнить · Esc закрыть"],
        UiText::MentionPickKeys => ["↑↓ 选择 · Enter 插入", "↑↓ 選擇 · Enter 插入", "↑↓ select · Enter insert", "↑↓ auswählen · Enter einfügen", "↑↓ elegir · Intro insertar", "↑↓ choisir · Entrée insérer", "↑↓ 選択 · Enter 挿入", "↑↓ 선택 · Enter 삽입", "↑↓ выбор · Enter вставить"],
        UiText::HelpMoreCommands => ["/help all 查看全部 · /help 命令名 查看用法", "/help all 查看全部 · /help 指令名 查看用法", "/help all for every command · /help <command> for details", "/help all für alle Befehle · /help <Befehl> für Details", "/help all para ver todo · /help <comando> para detalles", "/help all pour tout afficher · /help <commande> pour les détails", "/help all ですべて表示 · /help <コマンド> で使い方を表示", "/help all 전체 보기 · /help <명령> 사용법", "/help all — все команды · /help <команда> — подробности"],
        UiText::HelpEdit => ["编辑并重发本会话中的消息", "編輯並重送此對話中的訊息", "Edit and resend a message in this conversation", "Nachricht bearbeiten und erneut senden", "Editar y reenviar un mensaje", "Modifier et renvoyer un message", "会話のメッセージを編集して再送", "대화의 메시지를 수정하고 다시 보내기", "Изменить и отправить сообщение заново"],
        UiText::HelpCancelEdit => ["取消编辑，返回对话", "取消編輯並返回對話", "Cancel editing and return to chat", "Bearbeitung abbrechen", "Cancelar la edición", "Annuler la modification", "編集を取り消して会話に戻る", "편집 취소 후 대화로 돌아가기", "Отменить редактирование"],
        UiText::EditInputHint => ["编辑已发送消息 · Enter 重发 · Esc 取消", "編輯已傳送訊息 · Enter 重送 · Esc 取消", "Editing sent message · Enter to resend · Esc to cancel", "Nachricht bearbeiten · Enter erneut senden · Esc abbrechen", "Editar mensaje · Intro para reenviar · Esc para cancelar", "Modifier le message · Entrée pour renvoyer · Échap pour annuler", "送信済みメッセージを編集 · Enter で再送 · Esc でキャンセル", "보낸 메시지 편집 · Enter로 다시 보내기 · Esc로 취소", "Редактирование · Enter для отправки · Esc для отмены"],
        UiText::NoActiveEdit => ["当前没有正在编辑的消息。", "目前沒有正在編輯的訊息。", "No message is being edited.", "Keine Nachricht wird bearbeitet.", "No hay ningún mensaje en edición.", "Aucun message en cours de modification.", "編集中のメッセージはありません。", "편집 중인 메시지가 없습니다.", "Сейчас ни одно сообщение не редактируется."],
        UiText::NoMessageToEdit => ["此会话还没有可编辑的消息。", "此對話還沒有可編輯的訊息。", "No saved conversation to edit.", "Kein gespeichertes Gespräch zum Bearbeiten.", "No hay conversación guardada para editar.", "Aucune conversation enregistrée à modifier.", "編集できる会話がありません。", "편집할 저장된 대화가 없습니다.", "Нет сохранённого диалога для редактирования."],
        UiText::GoalInputHint => ["目标 · Enter 开始 · Alt+Enter 换行 · Esc 返回对话", "目標 · Enter 開始 · Alt+Enter 換行 · Esc 返回對話", "Goal · Enter start · Alt+Enter newline · Esc return to chat", "Ziel · Enter starten · Alt+Enter neue Zeile · Esc zurück", "Objetivo · Enter iniciar · Alt+Enter nueva línea · Esc volver", "Objectif · Entrée démarrer · Alt+Entrée nouvelle ligne · Échap retour", "目標 · Enter 開始 · Alt+Enter 改行 · Esc 会話に戻る", "목표 · Enter 시작 · Alt+Enter 줄 바꿈 · Esc 대화로 돌아가기", "Цель · Enter начать · Alt+Enter новая строка · Esc назад"],
        UiText::GoalPlaceholder => ["你希望 Nola 完成什么？", "您希望 Nola 完成什麼？", "What should Nola accomplish?", "Was soll Nola erledigen?", "¿Qué debe completar Nola?", "Que doit accomplir Nola ?", "Nola に何を達成してほしいですか？", "Nola가 무엇을 완료하길 원하시나요?", "Что должна выполнить Nola?"],
        UiText::GoalLifecycle => ["Nola 会持续推进，需要你提供信息或授权时暂停。/goal 查看状态 · /goal pause 暂停 · /goal resume 继续 · /goal off 结束目标", "Nola 會持續推進，需要您提供資訊或授權時暫停。/goal 檢視狀態 · /goal pause 暫停 · /goal resume 繼續 · /goal off 結束目標", "Nola keeps working and pauses for input or permission. /goal status · /goal pause · /goal resume · /goal off", "Nola arbeitet weiter und pausiert bei Rückfragen. /goal status · /goal pause · /goal resume · /goal off", "Nola continúa y se detiene para pedir información. /goal status · /goal pause · /goal resume · /goal off", "Nola continue et s’arrête pour une précision. /goal status · /goal pause · /goal resume · /goal off", "確認が必要な場合は一時停止します。/goal status · /goal pause · /goal resume · /goal off", "Nola가 계속 진행하며 입력이나 권한이 필요할 때 일시 중지합니다. /goal status · /goal pause · /goal resume · /goal off", "Nola продолжает работу и останавливается для уточнений. /goal status · /goal pause · /goal resume · /goal off"],
        UiText::Cancelled => ["已取消", "已取消", "Cancelled", "Abgebrochen", "Cancelada", "Annulée", "キャンセル済み", "취소됨", "Отменено"],
        UiText::ExitCode => ["退出码", "結束代碼", "Exit code", "Exit-Code", "Código de salida", "Code de sortie", "終了コード", "종료 코드", "Код выхода"],
        UiText::Using => ["正在使用", "正在使用", "Using", "Verwendet", "Usando", "Utilisation de", "使用中", "사용 중", "Используется"],
        UiText::SubagentPermissionRequest => ["子代理请求执行", "子代理請求執行", "Sub-agent requests", "Unteragent fordert an", "El subagente solicita", "Le sous-agent demande", "サブエージェントが要求", "하위 에이전트 요청", "Субагент запрашивает"],
        UiText::AllowResourceForTask => ["本任务允许此资源", "此任務允許此資源", "Allow this resource for this task", "Diese Ressource für diese Aufgabe erlauben", "Permitir este recurso para esta tarea", "Autoriser cette ressource pour cette tâche", "このタスクでこのリソースを許可", "이 작업에서 이 리소스 허용", "Разрешить этот ресурс для этой задачи"],
        UiText::AllowedTaskOnce => ["已允许任务执行一次", "已允許任務執行一次", "Allowed the task once", "Aufgabe einmalig zugelassen", "Tarea permitida una vez", "Tâche autorisée une fois", "タスクを一度許可", "작업을 한 번 허용함", "Задача разрешена один раз"],
        UiText::AllowedConversationSession => ["已允许本会话执行同类操作", "已允許此工作階段執行同類操作", "Allowed similar operations for this session", "Ähnliche Aktionen für diese Sitzung zugelassen", "Acciones similares permitidas para esta sesión", "Actions similaires autorisées pour cette session", "このセッションで同様の操作を許可しました", "이 세션에서 유사한 작업 허용됨", "Похожие действия разрешены для этого сеанса"],
        UiText::SentCustomTaskResponse => ["已向任务发送自定义回复", "已傳送自訂回覆至任務", "Sent a custom response to the task", "Eigene Antwort an die Aufgabe gesendet", "Respuesta personalizada enviada a la tarea", "Réponse personnalisée envoyée à la tâche", "タスクにカスタム回答を送信", "작업에 사용자 지정 응답을 보냈습니다", "Пользовательский ответ отправлен задаче"],
        UiText::DeniedTaskRequest => ["已拒绝任务请求", "已拒絕任務請求", "Denied the task request", "Aufgabenanfrage abgelehnt", "Solicitud de tarea rechazada", "Demande de tâche refusée", "タスク要求を拒否", "작업 요청 거부됨", "Запрос задачи отклонён"],
        UiText::Goal => ["目标", "目標", "Goal", "Ziel", "Objetivo", "Objectif", "目標", "목표", "Цель"],
        UiText::Items => ["项任务", "項", "items", "Einträge", "elementos", "éléments", "件", "개", "элементов"],
        UiText::Risks => ["注意", "注意", "Risks", "Risiken", "Riesgos", "Risques", "注意", "주의", "Риски"],
        UiText::Verify => ["验证", "驗證", "Verify", "Prüfen", "Verificar", "Vérifier", "検証", "검증", "Проверка"],
        UiText::ExecutionPlan => ["执行计划", "執行計畫", "Execution plan", "Ausführungsplan", "Plan de ejecución", "Plan d’exécution", "実行計画", "실행 계획", "План выполнения"],
    };
    values[match language {
        Language::ZhHans => 0,
        Language::ZhHant => 1,
        Language::En => 2,
        Language::De => 3,
        Language::Es => 4,
        Language::Fr => 5,
        Language::Ja => 6,
        Language::Ko => 7,
        Language::Ru => 8,
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
        Language::ZhHant => InteractionText {
            selection_hint: "↑/↓ 選擇 · Enter 確認 · Esc 取消",
            current_label: "目前：",
            question_input_prefix: "  輸入",
            question_or_direct_answer: "，或直接輸入回答",
            question_invalid_range_prefix: "  請輸入有效序號，範圍為 1-",
            question_invalid: "  請輸入有效序號，或輸入 q 取消",
            custom_answer: "自訂回答…",
            resume_title: "恢復工作階段",
            resume_inline_unsupported: "恢復工作階段模式不支援行內提示",
            resume_json_unsupported: "恢復工作階段模式不支援 --json",
            resume_terminal_only: "恢復工作階段模式僅支援互動式終端機",
            session_list: "工作階段清單",
            recent_sessions_hint: "最近工作階段 · ↑↓ 選擇 · Enter 恢復 · Esc 結束",
            resume_input_prompt: "輸入序號並按 Enter（輸入 q 結束）：",
            resume_invalid_prefix: "輸入無效，請輸入 1-",
            resume_invalid_suffix: "，或輸入 q 結束",
            empty_value: "<空>",
            no_execution_plan: "目前工作階段尚無執行計畫",
            execution_plan: "執行計畫",
            completed: "完成",
            complete: "已完成",
            in_progress: "進行中",
            pending: "待處理",
            current_project_session: "目前專案工作階段",
            loading_session_history: "載入工作階段歷史",
            messages_suffix: "則訊息",
            no_matches: "無符合項目",
            no_matches_in_window: "目前視窗無符合項目",
            confirmation_required: "需要使用者確認",
            decision_required: "需要您的選擇",
            question_cancelled: "使用者取消了選擇",
            question_input_required: "需要使用者輸入",
            answers_received_suffix: "個回答已收到",
            validation_pipeline_rejected: "驗證命令必須直接執行完成，不能透過 head 或 tail 截斷輸出",
            custom_permission_return: "已返回主輸入框，請繼續輸入",
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
        Language::Ko => InteractionText {
            selection_hint: "↑/↓ 이동 · Enter 확인 · Esc 취소",
            current_label: "현재:",
            question_input_prefix: "  입력",
            question_or_direct_answer: ", 또는 직접 답변 입력",
            question_invalid_range_prefix: "  유효한 번호를 입력하세요: 1-",
            question_invalid: "  유효한 번호를 입력하거나 q 로 취소",
            custom_answer: "사용자 지정 답변…",
            resume_title: "세션 재개",
            resume_inline_unsupported: "재개 모드에서는 인라인 입력을 사용할 수 없습니다",
            resume_json_unsupported: "재개 모드에서는 --json 을 지원하지 않습니다",
            resume_terminal_only: "재개 모드에는 대화형 터미널이 필요합니다",
            session_list: "세션 목록",
            recent_sessions_hint: "최근 세션 · ↑↓ 선택 · Enter 재개 · Esc 종료",
            resume_input_prompt: "번호를 입력하고 Enter를 누르세요 (q 로 종료):",
            resume_invalid_prefix: "잘못된 입력입니다. 1-",
            resume_invalid_suffix: " 또는 q 를 입력하세요",
            empty_value: "<비어 있음>",
            no_execution_plan: "이 세션에는 아직 실행 계획이 없습니다",
            execution_plan: "실행 계획",
            completed: "완료",
            complete: "완료됨",
            in_progress: "진행 중",
            pending: "대기 중",
            current_project_session: "현재 프로젝트 세션",
            loading_session_history: "세션 기록 불러오는 중",
            messages_suffix: "개 메시지",
            no_matches: "일치 항목 없음",
            no_matches_in_window: "현재 창에 일치 항목 없음",
            confirmation_required: "사용자 확인 필요",
            decision_required: "결정이 필요합니다",
            question_cancelled: "선택이 취소되었습니다",
            question_input_required: "사용자 입력 필요",
            answers_received_suffix: "개 답변 수신됨",
            validation_pipeline_rejected: "검증 명령은 head 또는 tail 로 출력을 자르지 않고 완료되어야 합니다",
            custom_permission_return: "기본 입력란으로 돌아왔습니다. 계속 입력하세요",
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
    if let Ok(guard) = LANGUAGE_OVERRIDE.read() {
        if let Some(language) = *guard {
            return language;
        }
    }

    for name in [
        "NOLA_CLI_LANG",
        "NOLA_LANG",
        "LANGUAGE",
        "LC_ALL",
        "LC_MESSAGES",
        "LANG",
    ] {
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
    } else if normalized == "zh-hant"
        || normalized.starts_with("zh-hant-")
        || normalized == "zh-tw"
        || normalized.starts_with("zh-tw-")
        || normalized == "zh-hk"
        || normalized.starts_with("zh-hk-")
        || normalized == "zh-mo"
        || normalized.starts_with("zh-mo-")
    {
        Some(Language::ZhHant)
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
    } else if normalized == "ko" || normalized.starts_with("ko-") {
        Some(Language::Ko)
    } else if normalized == "ru" || normalized.starts_with("ru-") {
        Some(Language::Ru)
    } else {
        None
    }
}

pub fn set_language(language: Language) {
    *LANGUAGE_OVERRIDE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(language);
}

#[cfg(test)]
pub(crate) fn clear_language_override_for_test() {
    *LANGUAGE_OVERRIDE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
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
        Language::ZhHant => "zh-TW",
        Language::En => "en-US",
        Language::De => "de-DE",
        Language::Es => "es-ES",
        Language::Fr => "fr-FR",
        Language::Ja => "ja-JP",
        Language::Ko => "ko-KR",
        Language::Ru => "ru-RU",
    }
}

pub fn browser_accept_language_for(language: Language) -> &'static str {
    match language {
        Language::ZhHans => "zh-CN,zh;q=0.9,en-US;q=0.8,en;q=0.7",
        Language::ZhHant => "zh-TW,zh-Hant;q=0.9,zh;q=0.8,en-US;q=0.7,en;q=0.6",
        Language::En => "en-US,en;q=0.9",
        Language::De => "de-DE,de;q=0.9,en-US;q=0.8,en;q=0.7",
        Language::Es => "es-ES,es;q=0.9,en-US;q=0.8,en;q=0.7",
        Language::Fr => "fr-FR,fr;q=0.9,en-US;q=0.8,en;q=0.7",
        Language::Ja => "ja-JP,ja;q=0.9,en-US;q=0.8,en;q=0.7",
        Language::Ko => "ko-KR,ko;q=0.9,en-US;q=0.8,en;q=0.7",
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
  nola web --connect-host <origin> --expected-host-id <id> --tailcat-helper <path> --tailcat-helper-sha256 <sha256> --token-stdin [--control-stdin] [--json]
  nola server [选项]
  nola server --host 127.0.0.1 --port <port> --tailcat-ingress [--tailcat-helper <path> --tailcat-helper-sha256 <sha256>]
  nola sessions
  nola config [show | edit [path] | check [path] | apply [path] [--dry-run] | example]
  nola room list | directory | create <title> | show <id> | join <room> <agent> | send <room> <text> [--to <participant>] | recover <room>
  nola skills [list|find <name-or-github-url>|install <github-url>|enable <id>|disable <id>]
  nola mcp [list|add <name> <target> [args...]|remove <id>|enable <id>|disable <id>]
  nola memory [list|add <text>|search <query>|create|revise|delete]
  nola model --help
  nola run --profile <id> [--profile-revision <revision>] [message]
  nola update [check|download <dir>]
  nola version [--json]
  nola plugin desktop-library
  nola plugin desktop-pages [id]
  nola plugin pi install <source> | list | pages | open | remove <source>
  nola plugin dsh [--profile tui|web] install <source> | list | pages | open | remove <name>
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <conversation-revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

选项：
  --endpoint URL              OpenAI 兼容端点，通常以 /v1 结尾
  --api-key KEY               API Key；为空时不发送
  --model, -m MODEL           发送给端点的模型名
  --project PATH              项目根目录，默认当前目录
  --allow-dir PATH            允许本地工具访问的目录；可重复
  --skill-dir PATH            额外的 Skill 目录；可重复
  --no-skills                 禁用文件系统 Skill 加载
  --context-file PATH         附加到提示词的上下文文件；可重复
  --expected-artifact REF     期望产物引用；可重复
  --bash-prefix PREFIX        允许执行的 Bash 命令前缀；可重复
  --json                      输出按行分隔的 JSON 事件
  --debug                     写入本地调试日志，并开启更详细终端交互（完整状态、工具调用与关键工具参数）
  --lang LANGUAGE             覆盖界面语言 (zh | zh-hant | en | de | es | fr | ja | ko | ru)
  --permission-profile MODE   权限配置：cautious 或 allow_all（覆盖 --yes/-y 的权限提升）
  --yes, -y                   自动确认 init/wiki 的模型联网请求，并启用 allow_all 工具权限（除非显式指定 --permission-profile）
  --network-policy MODE       联网策略：allow、ask、deny（默认 allow）
  --force-tool-set MODE       限制暴露工具：auto、all 或 write（默认 auto）
  --no-tools                  禁用本地工具
  --max-iterations N          限制可调用工具的模型请求轮次；0 表示不限
  --session ID                加载/保存指定会话
  --continue, -c              打开恢复会话页并继续最近保存的会话
  --resume                    打开恢复会话页（按↑/↓选择）并选择要继续的会话
  --no-save                   本次运行不保存到会话
  --output PATH               init 输出 NOLA.md 路径；wiki 输出 wiki 路径
  --force                     覆盖已存在的 init/wiki 输出文件
  --dry-run                   预览 init/wiki 输出或 config apply 的模型目录
  --host HOST                 web 服务监听地址，默认 127.0.0.1
  --port PORT                 web 服务端口，默认 5001
  --web-root PATH             web 资源目录，默认优先 <project>/apps/web，其次 <project>/web
  --version, -V               显示版本信息
  --help, -h                  显示帮助

环境变量：
  NOLA_CLI_LANG, NOLA_LANG
  NOLA_PERMISSION_PROFILE
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
        Language::ZhHant => {
            r#"Nola CLI

用法：
  nola [訊息...] [選項]
  nola init [選項]
  nola wiki [選項]
  nola web [選項]
  nola web --connect-host <origin> --expected-host-id <id> --tailcat-helper <path> --tailcat-helper-sha256 <sha256> --token-stdin [--control-stdin] [--json]
  nola server [選項]
  nola server --host 127.0.0.1 --port <port> --tailcat-ingress [--tailcat-helper <path> --tailcat-helper-sha256 <sha256>]
  nola sessions
  nola config [show | edit [path] | check [path] | apply [path] [--dry-run] | example]
  nola room list | directory | create <title> | show <id> | join <room> <agent> | send <room> <text> [--to <participant>] | recover <room>
  nola skills [list|find <name-or-github-url>|install <github-url>|enable <id>|disable <id>]
  nola mcp [list|add <name> <target> [args...]|remove <id>|enable <id>|disable <id>]
  nola memory [list|add <text>|search <query>|create|revise|delete]
  nola model --help
  nola run --profile <id> [--profile-revision <revision>] [message]
  nola update [check|download <dir>]
  nola version [--json]
  nola plugin desktop-library
  nola plugin desktop-pages [id]
  nola plugin pi install <source> | list | pages | open | remove <source>
  nola plugin dsh [--profile tui|web] install <source> | list | pages | open | remove <name>
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <conversation-revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

選項：
  --endpoint URL              OpenAI 相容端點，通常以 /v1 結尾
  --api-key KEY               API Key；為空時不傳送
  --model, -m MODEL           傳送給端點的模型名稱
  --project PATH              專案根目錄，預設為目前目錄
  --allow-dir PATH            允許本機工具存取的目錄；可重複指定
  --skill-dir PATH            額外的 Skill 目錄；可重複指定
  --no-skills                 停用檔案系統 Skill 載入
  --context-file PATH         附加到提示詞的上下文檔案；可重複指定
  --expected-artifact REF     期望產物參照；可重複指定
  --bash-prefix PREFIX        允許執行的 Bash 命令前綴；可重複指定
  --json                      輸出以換行分隔的 JSON 事件
  --debug                     寫入本機除錯日誌並開啟詳細終端互動
  --lang LANGUAGE             覆蓋介面語言 (zh | zh-hant | en | de | es | fr | ja | ko | ru)
  --permission-profile MODE   權限設定檔：cautious 或 allow_all（優先於 --yes/-y 的權限設定）
  --yes, -y                   自動核准 init/wiki 模型網路請求，並啟用 allow_all 工具權限（除非已明確指定 --permission-profile）
  --network-policy MODE       網路原則：allow、ask、deny（預設 allow）
  --force-tool-set MODE       限制公開的工具集合：auto、all 或 write（預設 auto）
  --no-tools                  停用本機工具
  --max-iterations N          限制可呼叫工具的模型請求輪次；0 表示不限
  --session ID                載入或儲存指定工作階段
  --continue, -c              開啟工作階段恢復頁並繼續最近儲存的工作階段
  --resume                    開啟工作階段恢復頁（按 ↑/↓ 選擇）並挑選要繼續的工作階段
  --no-save                   本次執行不儲存至工作階段
  --output PATH               init 輸出 NOLA.md 路徑；wiki 輸出 wiki 路徑
  --force                     覆蓋已存在的 init/wiki 輸出檔案
  --dry-run                   預覽 init/wiki 輸出或 config apply 的模型目錄
  --host HOST                 web 伺服器監聽位址，預設 127.0.0.1
  --port PORT                 web 伺服器連接埠，預設 5001
  --web-root PATH             web 資源目錄，預設依序尋找 <project>/apps/web 與 <project>/web
  --version, -V               顯示版本資訊
  --help, -h                  顯示說明

環境變數：
  NOLA_CLI_LANG, NOLA_LANG
  NOLA_PERMISSION_PROFILE
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

設定檔：
  ~/.nola/config.json(.jsonc)
  <project>/.nola/config.json(.jsonc), <project>/nola.json(.jsonc)
  相容 <project>/.nola/current_config.json(.jsonc)
  支援 llm、allowed_dirs、bash_prefixes、permissions、agents、compaction、external_execution、instructions、debug、language
"#
        }
        Language::En => {
            r#"Nola CLI

Usage:
  nola [message...] [options]
  nola init [options]
  nola wiki [options]
  nola web [options]
  nola web --connect-host <origin> --expected-host-id <id> --tailcat-helper <path> --tailcat-helper-sha256 <sha256> --token-stdin [--control-stdin] [--json]
  nola server [options]
  nola server --host 127.0.0.1 --port <port> --tailcat-ingress [--tailcat-helper <path> --tailcat-helper-sha256 <sha256>]
  nola sessions
  nola config [show | edit [path] | check [path] | apply [path] [--dry-run] | example]
  nola room list | directory | create <title> | show <id> | join <room> <agent> | send <room> <text> [--to <participant>] | recover <room>
  nola skills [list|find <name-or-github-url>|install <github-url>|enable <id>|disable <id>]
  nola mcp [list|add <name> <target> [args...]|remove <id>|enable <id>|disable <id>]
  nola memory [list|add <text>|search <query>|create|revise|delete]
  nola model --help
  nola run --profile <id> [--profile-revision <revision>] [message]
  nola update [check|download <dir>]
  nola version [--json]
  nola plugin desktop-library
  nola plugin desktop-pages [id]
  nola plugin pi install <source> | list | pages | open | remove <source>
  nola plugin dsh [--profile tui|web] install <source> | list | pages | open | remove <name>
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <conversation-revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

Options:
  --endpoint URL          OpenAI-compatible endpoint, usually ending in /v1
  --api-key KEY           API key; omitted when empty
  --model, -m MODEL       Model name sent to the endpoint
  --project PATH          Project root, default current directory
  --allow-dir PATH        Local directory tools may access; repeatable
  --skill-dir PATH        Additional skill directory to load; repeatable
  --no-skills             Disable filesystem skill loading
  --context-file PATH     Attach a context file to the prompt; repeatable
  --expected-artifact REF Expected artifact reference; repeatable
  --bash-prefix PREFIX    Bash command prefix to allow; repeatable
  --json                  Emit newline-delimited JSON events
  --debug                 Write local debug logs and enable detailed terminal interaction (full status + tool details)
  --lang LANGUAGE         Override interface language (zh | zh-hant | en | de | es | fr | ja | ko | ru)
  --permission-profile MODE   Permission profile: cautious|allow_all (overrides --yes/-y tool profile)
  --yes, -y               Approve init/wiki model-network requests and enable allow_all tool permissions (unless --permission-profile is set)
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
  --dry-run               Preview init/wiki output or the config apply catalog
  --host HOST             Web server bind host, default 127.0.0.1
  --port PORT             Web server port, default 5001
  --web-root PATH         Web asset root, default <project>/apps/web or <project>/web
  --version, -V           Show version information
  --help, -h              Show help

Environment:
  NOLA_CLI_LANG, NOLA_LANG
  NOLA_PERMISSION_PROFILE
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
  nola web --connect-host <origin> --expected-host-id <id> --tailcat-helper <path> --tailcat-helper-sha256 <sha256> --token-stdin [--control-stdin] [--json]
  nola server [Optionen]
  nola server --host 127.0.0.1 --port <port> --tailcat-ingress [--tailcat-helper <path> --tailcat-helper-sha256 <sha256>]
  nola sessions
  nola config [show | edit [path] | check [path] | apply [path] [--dry-run] | example]
  nola room list | directory | create <title> | show <id> | join <room> <agent> | send <room> <text> [--to <participant>] | recover <room>
  nola skills [list|find <name-or-github-url>|install <github-url>|enable <id>|disable <id>]
  nola mcp [list|add <name> <target> [args...]|remove <id>|enable <id>|disable <id>]
  nola memory [list|add <text>|search <query>|create|revise|delete]
  nola model --help
  nola run --profile <id> [--profile-revision <revision>] [message]
  nola update [check|download <dir>]
  nola version [--json]
  nola plugin desktop-library
  nola plugin desktop-pages [id]
  nola plugin pi install <source> | list | pages | open | remove <source>
  nola plugin dsh [--profile tui|web] install <source> | list | pages | open | remove <name>
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <conversation-revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

Optionen:
  --endpoint URL          OpenAI-kompatibler Endpunkt, meist mit /v1 am Ende
  --api-key KEY           API-Schlüssel; leer bedeutet nicht senden
  --model, -m MODEL           Modellname für den Endpunkt
  --project PATH              Projektstamm, standardmäßig das aktuelle Verzeichnis
  --allow-dir PATH            Verzeichnis, auf das lokale Tools zugreifen dürfen; wiederholbar
  --skill-dir PATH            Zusätzliches Skill-Verzeichnis; wiederholbar
  --no-skills                 Laden von Dateisystem-Skills deaktivieren
  --context-file PATH         Kontextdatei an den Prompt anhängen; wiederholbar
  --expected-artifact REF     Erwartete Artefaktreferenz; wiederholbar
  --bash-prefix PREFIX        Erlaubter Bash-Befehlspräfix; wiederholbar
  --json                      Zeilengetrennte JSON-Ereignisse ausgeben
  --debug                     Lokale Debug-Protokolle und ausführliche Terminaldetails aktivieren
  --lang LANGUAGE             Sprache der Oberfläche überschreiben (zh | zh-hant | en | de | es | fr | ja | ko | ru)
  --permission-profile MODE   Berechtigungsprofil: cautious|allow_all (überschreibt das Tool-Profil von --yes/-y)
  --yes, -y                   Modell-Netzwerkanfragen für init/wiki bestätigen und allow_all-Toolberechtigungen aktivieren (außer bei --permission-profile)
  --network-policy MODE       Netzwerkregel: allow|ask|deny (Standard: allow)
  --force-tool-set MODE       Sichtbare Tools begrenzen: auto|all|write (Standard: auto)
  --no-tools                  Lokale Tools deaktivieren
  --max-iterations N          Modellanfragen mit Tools begrenzen; 0 bedeutet unbegrenzt
  --session ID                Benannte Sitzung laden/speichern
  --continue, -c              Sitzungswiederherstellungsseite öffnen und zuletzt gespeicherte Sitzung fortsetzen
  --resume                    Sitzungswiederherstellungsseite öffnen (neueste zuerst) und eine Sitzung zum Fortsetzen wählen
  --no-save                   Diesen Lauf nicht als Sitzung speichern
  --output PATH               Ausgabepfad für init NOLA.md beziehungsweise wiki
  --force                     Vorhandene init/wiki-Ausgabedateien überschreiben
  --dry-run                   Vorschau für init/wiki oder den Katalog von config apply
  --host HOST                 Bind-Adresse des Webservers, Standard 127.0.0.1
  --port PORT                 Port des Webservers, Standard 5001
  --web-root PATH             Stammverzeichnis der Webressourcen, Standard <project>/apps/web oder <project>/web
  --version, -V               Version anzeigen
  --help, -h                  Hilfe anzeigen

Umgebung:
  NOLA_CLI_LANG, NOLA_LANG
  NOLA_PERMISSION_PROFILE
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
  nola web --connect-host <origin> --expected-host-id <id> --tailcat-helper <path> --tailcat-helper-sha256 <sha256> --token-stdin [--control-stdin] [--json]
  nola server [opciones]
  nola server --host 127.0.0.1 --port <port> --tailcat-ingress [--tailcat-helper <path> --tailcat-helper-sha256 <sha256>]
  nola sessions
  nola config [show | edit [path] | check [path] | apply [path] [--dry-run] | example]
  nola room list | directory | create <title> | show <id> | join <room> <agent> | send <room> <text> [--to <participant>] | recover <room>
  nola skills [list|find <name-or-github-url>|install <github-url>|enable <id>|disable <id>]
  nola mcp [list|add <name> <target> [args...]|remove <id>|enable <id>|disable <id>]
  nola memory [list|add <text>|search <query>|create|revise|delete]
  nola model --help
  nola run --profile <id> [--profile-revision <revision>] [message]
  nola update [check|download <dir>]
  nola version [--json]
  nola plugin desktop-library
  nola plugin desktop-pages [id]
  nola plugin pi install <source> | list | pages | open | remove <source>
  nola plugin dsh [--profile tui|web] install <source> | list | pages | open | remove <name>
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <conversation-revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

Opciones:
  --endpoint URL          Endpoint compatible con OpenAI, normalmente terminado en /v1
  --api-key KEY           Clave API; si está vacía no se envía
  --model, -m MODEL           Nombre del modelo enviado al endpoint
  --project PATH              Raíz del proyecto, por defecto el directorio actual
  --allow-dir PATH            Directorio accesible para herramientas locales; repetible
  --skill-dir PATH            Directorio adicional de Skills; repetible
  --no-skills                 Desactivar la carga de Skills del sistema de archivos
  --context-file PATH         Adjuntar un archivo de contexto al prompt; repetible
  --expected-artifact REF     Referencia de artefacto esperado; repetible
  --bash-prefix PREFIX        Prefijo de comando Bash permitido; repetible
  --json                      Emitir eventos JSON separados por líneas
  --debug                     Escribir registros locales y mostrar detalles completos en el terminal
  --lang LANGUAGE             Cambiar el idioma de la interfaz (zh | zh-hant | en | de | es | fr | ja | ko | ru)
  --permission-profile MODE   Perfil de permisos: cautious|allow_all (prevalece sobre --yes/-y)
  --yes, -y                   Aprobar red de init/wiki y activar permisos allow_all para herramientas (salvo --permission-profile)
  --network-policy MODE       Política de red: allow|ask|deny (predeterminado: allow)
  --force-tool-set MODE       Limitar herramientas visibles: auto|all|write (predeterminado: auto)
  --no-tools                  Desactivar herramientas locales
  --max-iterations N          Limitar solicitudes del modelo con herramientas; 0 sin límite
  --session ID                Cargar/guardar una sesión con nombre
  --continue, -c              Abrir la página de recuperación de sesión y continuar la última sesión guardada
  --resume                    Abrir la página de recuperación de sesión (de la más reciente a la más antigua) y elegir una para continuar
  --no-save                   No guardar esta ejecución en una sesión
  --output PATH               Ruta de salida de init NOLA.md o de wiki
  --force                     Sobrescribir los archivos de salida de init/wiki
  --dry-run                   Previsualizar init/wiki o el catálogo de config apply
  --host HOST                 Dirección del servidor web, predeterminada 127.0.0.1
  --port PORT                 Puerto del servidor web, predeterminado 5001
  --web-root PATH             Raíz de recursos web, predeterminada <project>/apps/web o <project>/web
  --version, -V               Mostrar la versión
  --help, -h                  Mostrar ayuda

Entorno:
  NOLA_CLI_LANG, NOLA_LANG
  NOLA_PERMISSION_PROFILE
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
  nola web --connect-host <origin> --expected-host-id <id> --tailcat-helper <path> --tailcat-helper-sha256 <sha256> --token-stdin [--control-stdin] [--json]
  nola server [options]
  nola server --host 127.0.0.1 --port <port> --tailcat-ingress [--tailcat-helper <path> --tailcat-helper-sha256 <sha256>]
  nola sessions
  nola config [show | edit [path] | check [path] | apply [path] [--dry-run] | example]
  nola room list | directory | create <title> | show <id> | join <room> <agent> | send <room> <text> [--to <participant>] | recover <room>
  nola skills [list|find <name-or-github-url>|install <github-url>|enable <id>|disable <id>]
  nola mcp [list|add <name> <target> [args...]|remove <id>|enable <id>|disable <id>]
  nola memory [list|add <text>|search <query>|create|revise|delete]
  nola model --help
  nola run --profile <id> [--profile-revision <revision>] [message]
  nola update [check|download <dir>]
  nola version [--json]
  nola plugin desktop-library
  nola plugin desktop-pages [id]
  nola plugin pi install <source> | list | pages | open | remove <source>
  nola plugin dsh [--profile tui|web] install <source> | list | pages | open | remove <name>
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <conversation-revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

Options :
  --endpoint URL          Point de terminaison compatible OpenAI, souvent terminé par /v1
  --api-key KEY           Clé API ; vide signifie ne pas l'envoyer
  --model, -m MODEL           Nom du modèle envoyé au point de terminaison
  --project PATH              Racine du projet, répertoire actuel par défaut
  --allow-dir PATH            Répertoire accessible aux outils locaux ; répétable
  --skill-dir PATH            Répertoire de Skills supplémentaire ; répétable
  --no-skills                 Désactiver le chargement des Skills du système de fichiers
  --context-file PATH         Joindre un fichier de contexte au prompt ; répétable
  --expected-artifact REF     Référence d'artefact attendu ; répétable
  --bash-prefix PREFIX        Préfixe de commande Bash autorisé ; répétable
  --json                      Émettre des événements JSON séparés par lignes
  --debug                     Écrire les journaux locaux et afficher les détails complets du terminal
  --lang LANGUAGE             Remplacer la langue de l'interface (zh | zh-hant | en | de | es | fr | ja | ko | ru)
  --permission-profile MODE   Profil d'autorisation : cautious|allow_all (prioritaire sur --yes/-y)
  --yes, -y                   Approuver le réseau init/wiki et activer le profil allow_all pour les outils (sauf si --permission-profile est défini)
  --network-policy MODE       Politique réseau : allow|ask|deny (allow par défaut)
  --force-tool-set MODE       Limiter les outils visibles : auto|all|write (auto par défaut)
  --no-tools                  Désactiver les outils locaux
  --max-iterations N          Limiter les requêtes du modèle avec outils ; 0 sans limite
  --session ID                Charger/enregistrer une session nommée
  --continue, -c              Ouvrir la page de reprise de session et continuer la dernière session enregistrée
  --resume                    Ouvrir la page de reprise de session (plus récentes d'abord) et en choisir une
  --no-save                   Ne pas enregistrer cette exécution dans une session
  --output PATH               Chemin de sortie de init NOLA.md ou du wiki
  --force                     Remplacer les fichiers de sortie init/wiki existants
  --dry-run                   Prévisualiser init/wiki ou le catalogue de config apply
  --host HOST                 Adresse d'écoute du serveur web, 127.0.0.1 par défaut
  --port PORT                 Port du serveur web, 5001 par défaut
  --web-root PATH             Racine des ressources web, <project>/apps/web ou <project>/web par défaut
  --version, -V               Afficher la version
  --help, -h                  Afficher l'aide

Environnement :
  NOLA_CLI_LANG, NOLA_LANG
  NOLA_PERMISSION_PROFILE
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
  nola web --connect-host <origin> --expected-host-id <id> --tailcat-helper <path> --tailcat-helper-sha256 <sha256> --token-stdin [--control-stdin] [--json]
  nola server [オプション]
  nola server --host 127.0.0.1 --port <port> --tailcat-ingress [--tailcat-helper <path> --tailcat-helper-sha256 <sha256>]
  nola sessions
  nola config [show | edit [path] | check [path] | apply [path] [--dry-run] | example]
  nola room list | directory | create <title> | show <id> | join <room> <agent> | send <room> <text> [--to <participant>] | recover <room>
  nola skills [list|find <name-or-github-url>|install <github-url>|enable <id>|disable <id>]
  nola mcp [list|add <name> <target> [args...]|remove <id>|enable <id>|disable <id>]
  nola memory [list|add <text>|search <query>|create|revise|delete]
  nola model --help
  nola run --profile <id> [--profile-revision <revision>] [message]
  nola update [check|download <dir>]
  nola version [--json]
  nola plugin desktop-library
  nola plugin desktop-pages [id]
  nola plugin pi install <source> | list | pages | open | remove <source>
  nola plugin dsh [--profile tui|web] install <source> | list | pages | open | remove <name>
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <conversation-revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

オプション:
  --endpoint URL          OpenAI 互換エンドポイント。通常は /v1 で終わります
  --api-key KEY           API キー。空の場合は送信しません
  --model, -m MODEL           エンドポイントへ送るモデル名
  --project PATH              プロジェクトルート。既定は現在のディレクトリ
  --allow-dir PATH            ローカルツールがアクセスできるディレクトリ。複数指定可
  --skill-dir PATH            追加の Skill ディレクトリ。複数指定可
  --no-skills                 ファイルシステムからの Skill 読み込みを無効化
  --context-file PATH         プロンプトに添付するコンテキストファイル。複数指定可
  --expected-artifact REF     期待される成果物参照。複数指定可
  --bash-prefix PREFIX        許可する Bash コマンド接頭辞。複数指定可
  --json                      改行区切りの JSON イベントを出力
  --debug                     ローカルデバッグログと詳細なターミナル表示を有効化
  --lang LANGUAGE             インターフェース言語の上書き (zh | zh-hant | en | de | es | fr | ja | ko | ru)
  --permission-profile MODE   権限プロファイル: cautious|allow_all (--yes/-y のツール権限設定を上書き)
  --yes, -y                   init/wiki のモデル通信を承認し allow_all ツール権限を有効化（--permission-profile 指定時を除く）
  --network-policy MODE       ネットワーク方針: allow|ask|deny (既定: allow)
  --force-tool-set MODE       公開ツールを制限: auto|all|write (既定: auto)
  --no-tools                  ローカルツールを無効化
  --max-iterations N          ツール利用可能なモデル要求を制限（0 は無制限）
  --session ID                名前付きセッションを読み込み/保存
  --continue, -c              セッション再開ページを開いて、最後に保存したセッションを続行
  --resume                    セッション再開ページを開いて（新しい順）続けるものを選択
  --no-save                   この実行をセッションに保存しない
  --output PATH               init の NOLA.md または wiki の出力先
  --force                     既存の init/wiki 出力ファイルを上書き
  --dry-run                   init/wiki の出力または config apply のカタログをプレビュー
  --host HOST                 Web サーバーの待受アドレス。既定 127.0.0.1
  --port PORT                 Web サーバーのポート。既定 5001
  --web-root PATH             Web リソースのルート。既定 <project>/apps/web または <project>/web
  --version, -V               バージョン情報を表示
  --help, -h                  ヘルプを表示

環境変数:
  NOLA_CLI_LANG, NOLA_LANG
  NOLA_PERMISSION_PROFILE
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
        Language::Ko => {
            r#"Nola CLI

사용법:
  nola [메시지...] [옵션]
  nola init [옵션]
  nola wiki [옵션]
  nola web [옵션]
  nola web --connect-host <origin> --expected-host-id <id> --tailcat-helper <path> --tailcat-helper-sha256 <sha256> --token-stdin [--control-stdin] [--json]
  nola server [옵션]
  nola server --host 127.0.0.1 --port <port> --tailcat-ingress [--tailcat-helper <path> --tailcat-helper-sha256 <sha256>]
  nola sessions
  nola config [show | edit [path] | check [path] | apply [path] [--dry-run] | example]
  nola room list | directory | create <title> | show <id> | join <room> <agent> | send <room> <text> [--to <participant>] | recover <room>
  nola skills [list|find <name-or-github-url>|install <github-url>|enable <id>|disable <id>]
  nola mcp [list|add <name> <target> [args...]|remove <id>|enable <id>|disable <id>]
  nola memory [list|add <text>|search <query>|create|revise|delete]
  nola model --help
  nola run --profile <id> [--profile-revision <revision>] [message]
  nola update [check|download <dir>]
  nola version [--json]
  nola plugin desktop-library
  nola plugin desktop-pages [id]
  nola plugin pi install <source> | list | pages | open | remove <source>
  nola plugin dsh [--profile tui|web] install <source> | list | pages | open | remove <name>
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <conversation-revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

옵션:
  --endpoint URL              OpenAI 호환 엔드포인트 (보통 /v1 로 끝남)
  --api-key KEY               API 키 (비어 있으면 전송하지 않음)
  --model, -m MODEL           엔드포인트에 전달할 모델 이름
  --project PATH              프로젝트 루트 (기본값: 현재 디렉터리)
  --allow-dir PATH            로컬 도구가 접근할 수 있는 디렉터리 (반복 가능)
  --skill-dir PATH            추가 Skill 디렉터리 (반복 가능)
  --no-skills                 파일 시스템 Skill 로드 비활성화
  --context-file PATH         프롬프트에 첨부할 컨텍스트 파일 (반복 가능)
  --expected-artifact REF     예상 산출물 참조 (반복 가능)
  --bash-prefix PREFIX        허용할 Bash 명령 접두사 (반복 가능)
  --json                      줄바꿈으로 구분된 JSON 이벤트 출력
  --debug                     로컬 디버그 로그 기록 및 상세 터미널 표시 활성화
  --lang LANGUAGE             인터페이스 언어 재정의 (zh | zh-hant | en | de | es | fr | ja | ko | ru)
  --permission-profile MODE   권한 프로필: cautious|allow_all (--yes/-y 도구 권한 설정보다 우선)
  --yes, -y                   init/wiki 모델 네트워크 요청 승인 및 allow_all 도구 권한 활성화 (--permission-profile 지정 시 제외)
  --network-policy MODE       네트워크 정책: allow|ask|deny (기본값: allow)
  --force-tool-set MODE       노출할 도구 제한: auto|all|write (기본값: auto)
  --no-tools                  로컬 도구 비활성화
  --max-iterations N          도구 호출 모델 요청 횟수 제한 (0 은 무제한)
  --session ID                지정한 세션 불러오기/저장
  --continue, -c              세션 복원 페이지를 열고 최근 저장된 세션 계속하기
  --resume                    세션 복원 페이지를 열고 (최신순) 이어서 할 세션 선택
  --no-save                   이번 실행을 세션에 저장하지 않음
  --output PATH               init NOLA.md 또는 wiki 출력 경로
  --force                     기존 init/wiki 출력 파일 덮어쓰기
  --dry-run                   init/wiki 출력 또는 config apply 카탈로그 미리보기
  --host HOST                 웹 서버 바인딩 호스트 (기본값: 127.0.0.1)
  --port PORT                 웹 서버 포트 (기본값: 5001)
  --web-root PATH             웹 리소스 루트 (기본값: <project>/apps/web 또는 <project>/web)
  --version, -V               버전 정보 표시
  --help, -h                  도움말 표시

환경 변수:
  NOLA_CLI_LANG, NOLA_LANG
  NOLA_PERMISSION_PROFILE
  NOLA_CLI_NETWORK_POLICY
  NOLA_CLI_PROJECT_ROOT
  NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD
  NOLA_AGENT_CONTEXT_WINDOW_TOKENS
  NOLA_CLI_ALLOWED_DIRS, NOLA_CLI_BASH_PREFIXES
  NOLA_CLI_DEBUG, NOLA_CLI_DEBUG_LOG_PATH
  NOLA_CLI_DEBUG_LOG_MAX_BYTES, NOLA_CLI_DEBUG_LOG_BACKUPS
  NOLA_CLI_INTERACTION_MODE (auto, always, never)
  NOLA_DB_PATH, NOLA_CLI_PYTHON

구성 파일:
  ~/.nola/config.json(.jsonc)
  <project>/.nola/config.json(.jsonc), <project>/nola.json(.jsonc)
  <project>/.nola/current_config.json(.jsonc) 와 호환
  llm, allowed_dirs, bash_prefixes, permissions, agents, compaction, external_execution, instructions, debug, language 지원
"#
        }
        Language::Ru => {
            r#"Nola CLI

Использование:
  nola [сообщение...] [параметры]
  nola init [параметры]
  nola wiki [параметры]
  nola web [параметры]
  nola web --connect-host <origin> --expected-host-id <id> --tailcat-helper <path> --tailcat-helper-sha256 <sha256> --token-stdin [--control-stdin] [--json]
  nola server [параметры]
  nola server --host 127.0.0.1 --port <port> --tailcat-ingress [--tailcat-helper <path> --tailcat-helper-sha256 <sha256>]
  nola sessions
  nola config [show | edit [path] | check [path] | apply [path] [--dry-run] | example]
  nola room list | directory | create <title> | show <id> | join <room> <agent> | send <room> <text> [--to <participant>] | recover <room>
  nola skills [list|find <name-or-github-url>|install <github-url>|enable <id>|disable <id>]
  nola mcp [list|add <name> <target> [args...]|remove <id>|enable <id>|disable <id>]
  nola memory [list|add <text>|search <query>|create|revise|delete]
  nola model --help
  nola run --profile <id> [--profile-revision <revision>] [message]
  nola update [check|download <dir>]
  nola version [--json]
  nola plugin desktop-library
  nola plugin desktop-pages [id]
  nola plugin pi install <source> | list | pages | open | remove <source>
  nola plugin dsh [--profile tui|web] install <source> | list | pages | open | remove <name>
  nola plugin inspect|install <directory>
  nola plugin list [--after <id>]
  nola plugin enable|disable|remove <id>
  nola plugin credential-set|credential-clear <id> <reference>
  nola session show <id>
  nola session part <conversation> <message> <part> <conversation-revision>
  nola session sources <conversation> [compaction-part]
  nola session delete <id>

Параметры:
  --endpoint URL              OpenAI-совместимый endpoint, обычно оканчивается на /v1
  --api-key KEY               API-ключ; если пустой, не отправляется
  --model, -m MODEL           Имя модели, отправляемое в endpoint
  --project PATH              Корень проекта, по умолчанию текущий каталог
  --allow-dir PATH            Каталог, доступный локальным инструментам; можно повторять
  --skill-dir PATH            Дополнительный каталог Skill; можно повторять
  --no-skills                 Отключить загрузку Skill из файловой системы
  --context-file PATH         Прикрепить файл контекста к запросу; можно повторять
  --expected-artifact REF     Ожидаемая ссылка на артефакт; можно повторять
  --bash-prefix PREFIX        Разрешенный префикс Bash-команды; можно повторять
  --json                      Выводить события JSON по одному на строку
  --debug                     Включить локальные журналы и подробный вывод терминала
  --lang LANGUAGE             Перекрывать язык интерфейса (zh | zh-hant | en | de | es | fr | ja | ko | ru)
  --permission-profile MODE   Профиль разрешений: cautious|allow_all (имеет приоритет над --yes/-y)
  --yes, -y                   Подтверждать сеть init/wiki и включать профиль allow_all для инструментов (если не задан --permission-profile)
  --network-policy MODE       Сетевая политика: allow|ask|deny (по умолчанию allow)
  --force-tool-set MODE       Ограничить доступные инструменты: auto|all|write (по умолчанию auto)
  --no-tools                  Отключить локальные инструменты
  --max-iterations N          Ограничить запросы модели с инструментами; 0 — без ограничения
  --session ID                Загрузить/сохранить именованный сеанс
  --continue, -c              Открыть страницу восстановления сессии и продолжить последний сохраненный сеанс
  --resume                    Открыть страницу восстановления сессии (сначала новые) и выбрать для продолжения
  --no-save                   Не сохранять этот запуск в сеанс
  --output PATH               Путь вывода init NOLA.md или wiki
  --force                     Перезаписать существующие файлы init/wiki
  --dry-run                   Предпросмотр init/wiki или каталога для config apply
  --host HOST                 Адрес веб-сервера, по умолчанию 127.0.0.1
  --port PORT                 Порт веб-сервера, по умолчанию 5001
  --web-root PATH             Корень веб-ресурсов, по умолчанию <project>/apps/web или <project>/web
  --version, -V               Показать версию
  --help, -h                  Показать справку

Окружение:
  NOLA_CLI_LANG, NOLA_LANG
  NOLA_PERMISSION_PROFILE
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
    if matches!(language, Language::ZhHans | Language::ZhHant | Language::Ja) {
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
        (Language::ZhHant, "answer_review_pending") => Some("待核驗正文"),
        (Language::ZhHant, "answer_review_approved") => Some("正文已核驗"),
        (Language::ZhHant, "answer_review_rejected") => Some("此版本未通過核驗，已失效"),
        (Language::ZhHant, "answer_review_interrupted") => Some("此版本核驗已中斷，請勿視為最終結論"),
        (Language::Ko, "answer_review_pending") => Some("초안 — 검토 대기 중"),
        (Language::Ko, "answer_review_approved") => Some("답변 검증 완료"),
        (Language::Ko, "answer_review_rejected") => Some("초안 거부됨 — 대체됨"),
        (Language::Ko, "answer_review_interrupted") => Some("검토 중단됨 — 최종 답변이 아닙니다"),
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
            Language::ZhHant => "正在核驗回答",
            Language::Ja => "回答を確認中",
            Language::Ko => "답변 검토 중",
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
            Language::ZhHant => "正在整理上下文",
            Language::Ja => "コンテキストを整理中",
            Language::Ko => "컨텍스트 압축 중",
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
        Language::ZhHant => match phase.as_str() {
            "diagnosing" => "正在診斷",
            "modifying" => "正在修改",
            "verifying" => "正在驗證",
            "correcting" => "正在修正",
            "awaiting_assessment" => "正在評估結果",
            _ => "正在處理",
        },
        Language::Ja => match phase.as_str() {
            "diagnosing" => "診断中",
            "modifying" => "変更中",
            "verifying" => "検証中",
            "correcting" => "修正中",
            "awaiting_assessment" => "結果を評価中",
            _ => "処理中",
        },
        Language::Ko => match phase.as_str() {
            "diagnosing" => "진단 중",
            "modifying" => "변경 적용 중",
            "verifying" => "검증 중",
            "correcting" => "수정 중",
            "awaiting_assessment" => "결과 평가 중",
            _ => "처리 중",
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

#[derive(serde::Deserialize)]
struct ToolNameCatalog {
    aliases: std::collections::HashMap<String, String>,
    labels: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
}

pub(crate) fn localize_tool_display_name_for(language: Language, name: &str) -> String {
    static CATALOG: std::sync::LazyLock<ToolNameCatalog> = std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!("../../../shared/localization/tool-names.json"))
            .expect("valid bundled tool name catalog")
    });
    let (locale, generic) = match language {
        Language::En => ("en", "Tool"),
        Language::ZhHans => ("zh-Hans", "工具"),
        Language::ZhHant => ("zh-Hant", "工具"),
        Language::De => ("de", "Werkzeug"),
        Language::Es => ("es", "Herramienta"),
        Language::Fr => ("fr", "Outil"),
        Language::Ja => ("ja", "ツール"),
        Language::Ko => ("ko", "도구"),
        Language::Ru => ("ru", "Инструмент"),
    };
    let name = name.trim();
    if name.is_empty() {
        return generic.to_owned();
    }
    let key = name.to_ascii_lowercase().replace('-', "_");
    let canonical = CATALOG.aliases.get(&key).unwrap_or(&key);
    CATALOG.labels.get(canonical).and_then(|labels| labels.get(locale)).cloned()
        .unwrap_or_else(|| format!("{generic} · {name}"))
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
        Language::ZhHant => zh_hant(message),
        Language::En => en(message),
        Language::De => de(message),
        Language::Es => es(message),
        Language::Fr => fr(message),
        Language::Ja => ja(message),
        Language::Ko => ko(message),
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
        Message::AbsentSessionPrefix => "Session already absent",
        Message::FailedReadPromptPrefix => "failed to read prompt:",
        Message::PromptRequired => "message is required",
        Message::SessionRequiresAction => "session requires an action: show or delete",
        Message::RequiresSessionIdSuffix => "requires a session id",
        Message::UnknownSessionActionPrefix => "unknown session action:",
        Message::UnknownCommandPrefix => "unknown command:",
        Message::MaxIterationsPositive => "--max-iterations must be a non-negative integer",
        Message::UnknownOptionPrefix => "unknown option:",
        Message::UnexpectedCommandArgs => "unexpected extra arguments for this command",
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
        Message::InteractiveExecutorUsage => "Usage: /executor <nola|codex|claude_code|opencode|antigravity>",
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
        Message::InteractiveGoalControlPending => "Goal control queued; use /goal status to check the applied state.",
        Message::InteractiveGoalAlreadyActive => "A Goal is already active. Use /goal resume, or /goal off before starting another.",
        Message::InteractiveSlashGoalUsage => "Usage: /goal [start] <objective> [-c/--constraints <constraints>] | /goal status | /goal pause | /goal resume | /goal off",
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
        Message::InteractiveWelcomeHint => "Type your first message · /help for commands",
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
        Message::AbsentSessionPrefix => "会话已不存在",
        Message::FailedReadPromptPrefix => "读取提示词失败：",
        Message::PromptRequired => "请输入消息内容",
        Message::SessionRequiresAction => "session 需要指定操作：show 或 delete",
        Message::RequiresSessionIdSuffix => "需要会话 ID",
        Message::UnknownSessionActionPrefix => "未知 session 操作：",
        Message::UnknownCommandPrefix => "未知命令：",
        Message::MaxIterationsPositive => "--max-iterations 必须是非负整数",
        Message::UnknownOptionPrefix => "未知选项：",
        Message::UnexpectedCommandArgs => "该命令不接受额外参数",
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
        Message::InteractiveExecutorUsage => "用法：/executor <nola|codex|claude_code|opencode|antigravity>",
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
        Message::InteractiveGoalControlPending => "目标操作已排队；用 /goal status 查看实际状态。",
        Message::InteractiveGoalAlreadyActive => "已有未结束的目标。用 /goal resume 继续，或先用 /goal off 取消再创建。",
        Message::InteractiveSlashGoalUsage => "用法： /goal [start] <objective> [-c/--constraints <constraints>] | /goal status | /goal pause | /goal resume | /goal off",
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
        Message::InteractiveWelcomeHint => "输入第一条消息 · /help 查看命令",
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
        Message::AbsentSessionPrefix => "Sitzung bereits entfernt",
        Message::FailedReadPromptPrefix => "Prompt konnte nicht gelesen werden:",
        Message::PromptRequired => "Nachricht erforderlich",
        Message::SessionRequiresAction => "session benötigt eine Aktion: show oder delete",
        Message::RequiresSessionIdSuffix => "benötigt eine Sitzungs-ID",
        Message::UnknownSessionActionPrefix => "unbekannte session-Aktion:",
        Message::UnknownCommandPrefix => "unbekannter Befehl:",
        Message::MaxIterationsPositive => "--max-iterations muss eine nichtnegative Ganzzahl sein",
        Message::UnknownOptionPrefix => "unbekannte Option:",
        Message::UnexpectedCommandArgs => "unerwartete Zusatzargumente für diesen Befehl",
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
        Message::InteractiveExecutorUsage => "Verwendung: /executor <nola|codex|claude_code|opencode|antigravity>",
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
        Message::InteractiveGoalControlPending => "Zielaktion vorgemerkt; Status mit /goal status prüfen.",
        Message::InteractiveGoalAlreadyActive => "Ein Ziel ist aktiv. /goal resume setzt es fort; /goal off beendet es.",
        Message::InteractiveSlashGoalUsage => "Verwendung: /goal [start] <objective> [-c/--constraints <constraints>] | /goal status | /goal pause | /goal resume | /goal off",
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
        Message::InteractiveWelcomeHint => "Erste Nachricht eingeben · Befehle mit /help",
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
        Message::AbsentSessionPrefix => "La sesión ya no existe",
        Message::FailedReadPromptPrefix => "no se pudo leer el prompt:",
        Message::PromptRequired => "se requiere un mensaje",
        Message::SessionRequiresAction => "session requiere una acción: show o delete",
        Message::RequiresSessionIdSuffix => "requiere un ID de sesión",
        Message::UnknownSessionActionPrefix => "acción de session desconocida:",
        Message::UnknownCommandPrefix => "comando desconocido:",
        Message::MaxIterationsPositive => "--max-iterations debe ser un entero no negativo",
        Message::UnknownOptionPrefix => "opción desconocida:",
        Message::UnexpectedCommandArgs => "argumentos extra inesperados para este comando",
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
        Message::InteractiveExecutorUsage => "Uso: /executor <nola|codex|claude_code|opencode|antigravity>",
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
        Message::InteractiveGoalControlPending => "Control del objetivo en cola; consulta /goal status.",
        Message::InteractiveGoalAlreadyActive => "Ya hay un objetivo activo. Usa /goal resume o /goal off antes de crear otro.",
        Message::InteractiveSlashGoalUsage => "Uso: /goal [start] <objective> [-c/--constraints <constraints>] | /goal status | /goal pause | /goal resume | /goal off",
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
        Message::InteractiveWelcomeHint => "Escribe tu primer mensaje · /help para ver los comandos",
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
        Message::AbsentSessionPrefix => "La session est déjà absente",
        Message::FailedReadPromptPrefix => "échec de lecture du prompt :",
        Message::PromptRequired => "un message est requis",
        Message::SessionRequiresAction => "session nécessite une action : show ou delete",
        Message::RequiresSessionIdSuffix => "nécessite un ID de session",
        Message::UnknownSessionActionPrefix => "action session inconnue :",
        Message::UnknownCommandPrefix => "commande inconnue :",
        Message::MaxIterationsPositive => "--max-iterations doit être un entier non négatif",
        Message::UnknownOptionPrefix => "option inconnue :",
        Message::UnexpectedCommandArgs => {
            "arguments supplémentaires inattendus pour cette commande"
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
        Message::InteractiveExecutorUsage => "Utilisation : /executor <nola|codex|claude_code|opencode|antigravity>",
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
        Message::InteractiveGoalControlPending => "Action en attente ; vérifiez avec /goal status.",
        Message::InteractiveGoalAlreadyActive => "Un objectif existe. Utilisez /goal resume ou /goal off avant d’en créer un autre.",
        Message::InteractiveSlashGoalUsage => "Utilisation : /goal [start] <objective> [-c/--constraints <constraints>] | /goal status | /goal pause | /goal resume | /goal off",
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
        Message::InteractiveWelcomeHint => "Écrivez votre premier message · commandes avec /help",
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
        Message::AbsentSessionPrefix => "セッションは既に存在しません",
        Message::FailedReadPromptPrefix => "プロンプトの読み取りに失敗しました:",
        Message::PromptRequired => "メッセージを入力してください",
        Message::SessionRequiresAction => "session には操作 show または delete が必要です",
        Message::RequiresSessionIdSuffix => "にはセッション ID が必要です",
        Message::UnknownSessionActionPrefix => "不明な session 操作:",
        Message::UnknownCommandPrefix => "不明なコマンド:",
        Message::MaxIterationsPositive => "--max-iterations は 0 以上の整数である必要があります",
        Message::UnknownOptionPrefix => "不明なオプション:",
        Message::UnexpectedCommandArgs => "このコマンドに予期しない追加引数があります",
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
        Message::InteractiveExecutorUsage => "使用法: /executor <nola|codex|claude_code|opencode|antigravity>",
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
        Message::InteractiveGoalControlPending => "目標の操作を受付済みです。/goal status で状態を確認してください。",
        Message::InteractiveGoalAlreadyActive => "未完了の目標があります。/goal resume で再開するか、/goal off で終了してください。",
        Message::InteractiveSlashGoalUsage => "使用法: /goal [start] <objective> [-c/--constraints <constraints>] | /goal status | /goal pause | /goal resume | /goal off",
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
        Message::InteractiveWelcomeHint => "メッセージを入力 · コマンド一覧は /help",
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
        Message::AbsentSessionPrefix => "Сессия уже отсутствует",
        Message::FailedReadPromptPrefix => "не удалось прочитать запрос:",
        Message::PromptRequired => "требуется сообщение",
        Message::SessionRequiresAction => "session требует действие: show или delete",
        Message::RequiresSessionIdSuffix => "требует ID сеанса",
        Message::UnknownSessionActionPrefix => "неизвестное действие session:",
        Message::UnknownCommandPrefix => "неизвестная команда:",
        Message::MaxIterationsPositive => "--max-iterations должен быть неотрицательным целым числом",
        Message::UnknownOptionPrefix => "неизвестный параметр:",
        Message::UnexpectedCommandArgs => "лишние аргументы для этой команды",
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
        Message::InteractiveExecutorUsage => "Использование: /executor <nola|codex|claude_code|opencode|antigravity>",
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
        Message::InteractiveGoalControlPending => "Управление целью в очереди; проверьте /goal status.",
        Message::InteractiveGoalAlreadyActive => "Цель уже существует. Используйте /goal resume или /goal off перед созданием новой.",
        Message::InteractiveSlashGoalUsage => "Использование: /goal [start] <objective> [-c/--constraints <constraints>] | /goal status | /goal pause | /goal resume | /goal off",
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
        Message::InteractiveWelcomeHint => "Введите сообщение · список команд: /help",
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

fn zh_hant(message: Message) -> &'static str {
    match message {
        Message::ErrorPrefix => "錯誤：",
        Message::NoSavedSessions => "尚無已儲存的工作階段。",
        Message::MessagesUnit => "則訊息",
        Message::SessionLabel => "工作階段：",
        Message::CreatedLabel => "建立時間：",
        Message::UpdatedLabel => "更新時間：",
        Message::MessagesLabel => "訊息數：",
        Message::DeletedSessionPrefix => "已刪除工作階段",
        Message::AbsentSessionPrefix => "會話已不存在",
        Message::FailedReadPromptPrefix => "讀取提示詞失敗：",
        Message::PromptRequired => "必須輸入訊息內容",
        Message::SessionRequiresAction => "session 需要指定操作：show 或 delete",
        Message::RequiresSessionIdSuffix => "需要工作階段 ID",
        Message::UnknownSessionActionPrefix => "未知的 session 操作：",
        Message::UnknownCommandPrefix => "未知命令：",
        Message::MaxIterationsPositive => "--max-iterations 必須為非負整數",
        Message::UnknownOptionPrefix => "未知選項：",
        Message::UnexpectedCommandArgs => "此命令不接受額外引數",
        Message::MissingEndpoint => "缺少 --endpoint 或 Nola 設定檔中的 llm.endpoint",
        Message::ContinueOrSession => "--continue 與 --session 僅能擇一使用",
        Message::CurrentDirFailedPrefix => "讀取目前目錄失敗：",
        Message::RequiresValueSuffix => "需要提供值",
        Message::FailedResolvePathPrefix => "解析路徑失敗",
        Message::ActiveSessionLabel => "工作階段：",
        Message::FailedCreateSessionDirPrefix => "建立工作階段目錄失敗：",
        Message::FailedReadSessionPrefix => "讀取工作階段失敗",
        Message::FailedParseSessionPrefix => "解析工作階段失敗",
        Message::FailedSerializeSessionPrefix => "序列化工作階段失敗：",
        Message::FailedSaveSessionPrefix => "儲存工作階段失敗：",
        Message::FailedReadSessionsPrefix => "讀取工作階段清單失敗：",
        Message::FailedReadSessionEntryPrefix => "讀取工作階段項目失敗：",
        Message::FailedDeleteSessionPrefix => "刪除工作階段失敗",
        Message::HomeNotSet => "未設定 HOME，無法儲存工作階段",
        Message::FailedReadLatestSessionPrefix => "讀取最近工作階段失敗：",
        Message::FailedSaveLatestSessionPrefix => "儲存最近工作階段失敗：",
        Message::InvalidSessionId => "工作階段 ID 僅能包含字母、數字、'-' 與 '_'",
        Message::FailedCreateHttpClientPrefix => "建立 HTTP 用戶端失敗：",
        Message::LlmRequestFailedPrefix => "LLM 請求失敗",
        Message::LlmStreamReadFailedPrefix => "讀取 LLM 串流失敗：",
        Message::ToolRequiresApprovalPrefix => "工具",
        Message::ToolRequiresApprovalSuffix => "需要核准。",
        Message::ArgsLabel => "引數：",
        Message::AllowOncePrompt => "允許本次執行？[y/N] ",
        Message::PermissionDeniedByUser => "使用者已拒絕授權",
        Message::PermissionDeniedGuidance => {
            "權限未核准，操作未執行。輸入 /allow 可檢視權限設定；設定檔中的明確 deny 規則需先調整設定後再明確要求重試。系統權限約束依然有效。"
        }
        Message::SubAgentLabel => "子代理：",
        Message::TaskLabel => "任務：",
        Message::CapabilityLabel => "能力：",
        Message::PromptLabel => "提示詞：",
        Message::DelegateAgentCapability => "可在已設定的權限內執行命令或修改檔案",
        Message::DelegateAgentConfirmationRequired => {
            "需要明確核准子代理；其每個工具仍須遵守各自的資源權限。"
        }
        Message::NetworkAskNonInteractive => {
            "非互動模式下已封鎖網路請求（policy=ask）。若要繼續，請改用 --network-policy=allow。"
        }
        Message::NetworkAskPrompt => "是否允許本次模型網路請求？[y/N]: ",
        Message::NetworkAllowedByUser => "使用者已允許本次模型網路請求。",
        Message::NetworkDeniedByUser => "使用者已拒絕本次模型網路請求。",
        Message::DisplayReadPrefix => "已讀取",
        Message::DisplayWrotePrefix => "已寫入",
        Message::DisplayEditedPrefix => "已編輯",
        Message::DisplayListedPrefix => "已列出",
        Message::EntriesUnit => "項",
        Message::MatchesUnit => "筆符合",
        Message::ReachedMaxIterationsPrefix => "\n\n已達到最大輪次（",
        Message::ReachedMaxIterationsSuffix => "），傳回目前結果。",
        Message::InteractiveQueuedRequest => "  已排隊請求：",
        Message::InteractiveRunning => "  執行中...",
        Message::InteractiveReadyForNextRequest => "  已就緒，可處理下一個請求",
        Message::InteractiveModelLabel => "模型",
        Message::InteractiveParamsLabel => "參數",
        Message::InteractiveWorkingDirLabel => "工作目錄",
        Message::InteractiveWorkspaceLabel => "工作區",
        Message::InteractiveProviderLabel => "提供者",
        Message::InteractiveProviderLocal => "本機設定",
        Message::InteractiveModeFieldLabel => "模式",
        Message::InteractiveNetworkFieldLabel => "網路",
        Message::InteractiveEndpointLabel => "端點",
        Message::InteractiveAvailableModels => "可用模型：",
        Message::InteractiveModelUsageHint => "用法：/model <name>",
        Message::InteractiveAvailableCommands => "可用命令：",
        Message::InteractiveSlashHelpHint => "顯示說明",
        Message::InteractiveSlashModelHint => "列出模型，/model <name> 切換模型",
        Message::InteractiveSlashExecutorHint => "列出或切換核心執行單元",
        Message::InteractiveAvailableExecutors => "執行單元：",
        Message::InteractiveExecutorUsage => "用法：/executor <nola|codex|claude_code|opencode|antigravity>",
        Message::InteractiveExecutorSelected => "已選擇執行單元：",
        Message::InteractiveExecutorUnavailable => "執行單元無法使用：",
        Message::InteractiveExecutorLabel => "執行單元",
        Message::InteractiveSlashDebugHint => "顯示或切換除錯詳細資訊",
        Message::InteractiveSlashDebugUsage => "用法：/debug on | /debug off",
        Message::InteractiveSlashAllowHint => "設定權限模式",
        Message::InteractiveSlashAllowUsage => "用法：/allow — 開啟權限選擇面板",
        Message::InteractiveAllowPanelTitle => "權限模式",
        Message::InteractiveAllowPanelCautious => "謹慎",
        Message::InteractiveAllowPanelCautiousDesc => "刪除、覆寫、敏感資料存取、對外傳輸與新連線前先確認；一般讀取與新增檔案直接執行。",
        Message::InteractiveAllowPanelAllowAll => "全部允許",
        Message::InteractiveAllowPanelAllowAllDesc => "此工作階段內無需 Nola 確認提示，系統權限約束依然適用。",
        Message::InteractiveAllowPanelSelected => "已選擇：",
        Message::InteractiveAllowPanelCancelled => "已取消",
        Message::InteractiveSlashGoalHint => "自主推進並達成目標",
        Message::InteractiveGoalControlPending => "目標操作已排入佇列；請用 /goal status 檢視實際狀態。",
        Message::InteractiveGoalAlreadyActive => "已有進行中的目標。請用 /goal resume 繼續，或先以 /goal off 結束後再啟動新目標。",
        Message::InteractiveSlashGoalUsage => "用法：/goal [start] <objective> [-c/--constraints <constraints>] | /goal status | /goal pause | /goal resume | /goal off",
        Message::InteractiveSlashInitHint => "產生 NOLA.md 專案記憶（--dry-run、--force、--output PATH）",
        Message::InteractiveSlashCdHint => "切換工作區並開啟新的專案工作階段（/cd - 返回先前的目錄）",
        Message::InteractiveSlashCompactHint => "精簡模型上下文，同時保留完整重播歷史",
        Message::InteractiveSlashNewHint => "在目前工作區開啟全新工作階段",
        Message::InteractiveSlashPlanHint => "顯示目前工作階段的執行計畫",
        Message::InteractiveSlashJobsHint => "列出、檢視或停止持久化背景任務",
        Message::InteractiveSlashResumeHint => "開啟工作階段恢復頁面",
        Message::InteractiveSlashContinueHint => "繼續目前的專案最新工作階段",
        Message::InteractiveSlashHistoryHint => "重新顯示完整工作階段歷史",
        Message::InteractiveSlashStatusHint => "顯示目前執行狀態",
        Message::InteractiveSlashExitHint => "結束目前工作階段",
        Message::InteractiveSlashQuitHint => "q/quit/exit 亦可結束",
        Message::InteractiveWelcomeTagline => "本機優先的個人助手",
        Message::InteractiveWelcomeHint => "輸入第一則訊息 · /help 查看指令",
        Message::InteractiveAutoValue => "自動",
        Message::InteractiveNetworkAllowValue => "允許",
        Message::InteractiveNetworkAskValue => "詢問",
        Message::InteractiveNetworkDenyValue => "禁止",
        Message::InteractiveContextLabel => "上下文",
        Message::InteractiveTokensLabel => "權杖",
        Message::InteractiveTokensUnit => "權杖",
        Message::InteractiveMessagesUnit => "則訊息",
        Message::InteractiveAllowPanelErrorPrefix => "權限面板錯誤：",
        Message::InteractiveGoalStatusPrefix => "目標：",
        Message::InteractiveGoalStatePrefix => "狀態：",
        Message::InteractiveGoalStateRunning => "執行中",
        Message::InteractiveGoalStatePaused => "已暫停",
        Message::InteractiveGoalStateNeedsUser => "等待使用者",
        Message::InteractiveGoalStateFailed => "執行失敗",
        Message::InteractiveGoalResumeStarted => "正在恢復目標：",
        Message::InteractiveGoalNoResumable => "目前工作階段沒有可恢復的目標。",
        Message::InteractiveWorkspaceStatusPrefix => "工作區：",
        Message::InteractiveCdUsage => "用法：/cd PATH · /cd - 返回先前的目錄",
        Message::InteractiveCompactTitle => "上下文壓縮",
        Message::InteractiveCompactNoActiveSession => "目前沒有作用中的工作階段",
        Message::InteractiveCompactAlready => "已完成壓縮",
        Message::InteractiveCompactCompleted => "壓縮完成",
        Message::InteractiveCompactSummarized => "已摘要",
        Message::InteractiveCompactKeptRecent => "保留最近",
        Message::InteractiveCompactSaved => "已節省",
        Message::InteractiveCompactHistoryPreserved => "完整對話紀錄仍會保留以供重播",
        Message::InteractiveCompactFailedPrefix => "上下文壓縮失敗：",
        Message::InteractiveNewSessionTitle => "新工作階段",
        Message::InteractiveNewSessionPending => "下一則訊息將開始全新工作階段",
        Message::InteractiveJobsTitle => "背景任務",
        Message::InteractiveJobsUsage => "用法：/jobs · /jobs ID · /jobs status ID · /jobs stop ID",
        Message::InteractiveUnknownCommandHint => "未知命令；請輸入 /help 檢視可用命令",
        Message::InteractiveChoiceLabel => "請選擇",
        Message::InteractiveModelStatusPrefix => "模型：",
        Message::InteractiveDebugStatusPrefix => "除錯：",
        Message::InteractiveAllowStatusPrefix => "權限：",
        Message::InteractiveDefaultModelLabel => "(預設)",
        Message::InteractiveTurnsLabel => "輪次",
        Message::InteractiveToolsLabel => "工具",
        Message::InteractiveNetworkLabel => "網路",
        Message::InteractiveInputPlaceholder => "輸入訊息…",
        Message::InteractiveExecutionDraftPrefix => "↳",
        Message::InteractiveExecutionQueuedPrefix => "已排隊補充指示：",
        Message::InteractiveExecutionInterruptHint => "按 Esc 中斷目前執行",
        Message::InteractiveExecutionInputHint => "Esc 中斷 · Enter 排隊補充指示",
        Message::AgentStatusThinking => "正在思考",
        Message::AgentStatusRetryingModel => "正在重試模型回應",
        Message::AgentStatusStrictToolStrategy => "嚴格工具原則",
        Message::AgentStatusChooseFromAvailableTools => "從可用工具中選取",
        Message::AgentStatusReadyToContinue => "已就緒，等待下一步",
        Message::AgentStatusEnteredMode => "已進入模式",
        Message::AgentStatusReplyingDirectly => "準備直接回覆",
        Message::AgentStatusExecutingPlan => "正在執行計畫",
        Message::AgentStatusWaitingForUserConfirmation => "等待使用者確認",
        Message::AgentStatusProcessing => "處理中",
        Message::AgentStatusPreparing => "正在準備",
        Message::AgentStatusModelUnavailable => "模型暫時無法使用",
        Message::AgentDoneLabel => "完成",
        Message::AgentErrorLabel => "錯誤",
        Message::AgentToolLabel => "工具",
        Message::AgentAgentLabel => "代理",
        Message::AgentSessionLabel => "工作階段",
        Message::AgentStatusLabel => "狀態",
        Message::AgentInfoLabel => "資訊",
        Message::AgentReadyCurrentModePrefix => "已就緒，目前為",
        Message::AgentReadyCurrentModeSuffix => "模式",
        Message::AgentStatusDetailsSeparator => "、",
        Message::AgentPreparingToExecutePrefix => "準備執行",
        Message::AgentActionCountSuffixSingle => "個動作",
        Message::AgentActionCountSuffixPlural => "個動作",
        Message::AgentTurnPrefix => "第",
        Message::AgentTurnSuffix => "輪",
        Message::AgentInspectingProject => "正在檢視專案",
        Message::AgentEditingFiles => "正在修改檔案",
        Message::AgentRunningCommand => "正在執行命令",
        Message::AgentAccessingNetwork => "正在存取網路",
        Message::AgentSearchingTools => "正在尋找工具",
        Message::AgentStartingSubtask => "正在啟動子任務",
        Message::AgentFileModified => "已修改檔案",
        Message::AgentCommandCompleted => "命令已完成",
        Message::AgentNetworkRequestCompleted => "網路請求已完成",
        Message::AgentSubtaskCompleted => "子任務已完成",
        Message::AgentFailedToModifyFile => "無法修改檔案",
        Message::AgentCommandFailed => "命令執行失敗",
        Message::AgentNetworkRequestFailed => "網路請求失敗",
        Message::AgentSubtaskFailed => "子任務失敗",
        Message::AgentOperationFailed => "操作失敗",
        Message::InteractiveUnknownPath => "(未知)",
        Message::InteractiveStatusOn => "開啟",
        Message::InteractiveStatusOff => "關閉",
        Message::InteractiveInvalidLanguageValue => "不支援的語言",
        Message::InvalidPortValue => "--port 必須是 1 到 65535 之間的數字",
        Message::UnknownAgentModePrefix => "未知的 --agent-mode",
        Message::NetworkPolicyMustBeOneOf => "--network-policy 僅能為 allow、ask、deny 之一",
        Message::ForceToolSetMustBeOneOf => "--force-tool-set 僅能為 auto、all、write 之一",
        Message::FailedReadKeyEventPrefix => "讀取按鍵事件失敗：",
        Message::FailedFlushPromptPrefix => "重新整理輸入提示失敗：",
        Message::InteractiveRawModeFailedPrefix => "開啟原始輸入模式失敗：",
    }
}

fn ko(message: Message) -> &'static str {
    match message {
        Message::ErrorPrefix => "오류:",
        Message::NoSavedSessions => "저장된 세션이 없습니다.",
        Message::MessagesUnit => "개 메시지",
        Message::SessionLabel => "세션:",
        Message::CreatedLabel => "생성 시각:",
        Message::UpdatedLabel => "업데이트 시각:",
        Message::MessagesLabel => "메시지 수:",
        Message::DeletedSessionPrefix => "세션 삭제됨",
        Message::AbsentSessionPrefix => "세션이 이미 없습니다",
        Message::FailedReadPromptPrefix => "프롬프트 읽기 실패:",
        Message::PromptRequired => "메시지 내용이 필요합니다",
        Message::SessionRequiresAction => "session 은 작업을 지정해야 합니다: show 또는 delete",
        Message::RequiresSessionIdSuffix => "세션 ID 가 필요합니다",
        Message::UnknownSessionActionPrefix => "알 수 없는 session 작업:",
        Message::UnknownCommandPrefix => "알 수 없는 명령:",
        Message::MaxIterationsPositive => "--max-iterations 는 0 이상의 정수여야 합니다",
        Message::UnknownOptionPrefix => "알 수 없는 옵션:",
        Message::UnexpectedCommandArgs => "이 명령은 추가 인수를 받지 않습니다",
        Message::MissingEndpoint => "--endpoint 또는 Nola 설정 파일의 llm.endpoint 가 누락되었습니다",
        Message::ContinueOrSession => "--continue 와 --session 은 동시에 사용할 수 없습니다",
        Message::CurrentDirFailedPrefix => "현재 디렉터리 확인 실패:",
        Message::RequiresValueSuffix => "값이 필요합니다",
        Message::FailedResolvePathPrefix => "경로 확인 실패",
        Message::ActiveSessionLabel => "세션:",
        Message::FailedCreateSessionDirPrefix => "세션 디렉터리 생성 실패:",
        Message::FailedReadSessionPrefix => "세션 읽기 실패",
        Message::FailedParseSessionPrefix => "세션 구문 분석 실패",
        Message::FailedSerializeSessionPrefix => "세션 직렬화 실패:",
        Message::FailedSaveSessionPrefix => "세션 저장 실패:",
        Message::FailedReadSessionsPrefix => "세션 목록 읽기 실패:",
        Message::FailedReadSessionEntryPrefix => "세션 항목 읽기 실패:",
        Message::FailedDeleteSessionPrefix => "세션 삭제 실패",
        Message::HomeNotSet => "HOME 이 설정되지 않아 세션을 저장할 수 없습니다",
        Message::FailedReadLatestSessionPrefix => "최근 세션 읽기 실패:",
        Message::FailedSaveLatestSessionPrefix => "최근 세션 저장 실패:",
        Message::InvalidSessionId => "세션 ID 는 영문자, 숫자, '-' 및 '_' 만 포함할 수 있습니다",
        Message::FailedCreateHttpClientPrefix => "HTTP 클라이언트 생성 실패:",
        Message::LlmRequestFailedPrefix => "LLM 요청 실패",
        Message::LlmStreamReadFailedPrefix => "LLM 스트림 읽기 실패:",
        Message::ToolRequiresApprovalPrefix => "도구",
        Message::ToolRequiresApprovalSuffix => "실행에 승인이 필요합니다.",
        Message::ArgsLabel => "인수:",
        Message::AllowOncePrompt => "이번 한 번 허용하시겠습니까? [y/N] ",
        Message::PermissionDeniedByUser => "사용자가 권한을 거부했습니다",
        Message::PermissionDeniedGuidance => {
            "권한이 거부되어 작업이 실행되지 않았습니다. /allow 로 권한을 확인하세요. 설정의 명시적 deny 규칙을 변경한 뒤 다시 시도해 달라고 요청하세요. 시스템 권한 제한은 계속 적용됩니다."
        }
        Message::SubAgentLabel => "하위 에이전트:",
        Message::TaskLabel => "작업:",
        Message::CapabilityLabel => "기능:",
        Message::PromptLabel => "프롬프트:",
        Message::DelegateAgentCapability => "설정된 권한 내에서 명령을 실행하거나 파일을 수정할 수 있습니다",
        Message::DelegateAgentConfirmationRequired => {
            "하위 에이전트에 대한 명시적 승인이 필요합니다. 각 도구는 여전히 개별 리소스 권한의 적용을 받습니다."
        }
        Message::NetworkAskNonInteractive => {
            "비대화형 모드에서 네트워크 요청이 차단되었습니다(policy=ask). 계속하려면 --network-policy=allow 를 사용하세요."
        }
        Message::NetworkAskPrompt => "이번 모델 네트워크 요청을 허용하시겠습니까? [y/N]: ",
        Message::NetworkAllowedByUser => "사용자가 이번 모델 네트워크 요청을 허용했습니다.",
        Message::NetworkDeniedByUser => "사용자가 이번 모델 네트워크 요청을 거부했습니다.",
        Message::DisplayReadPrefix => "읽음",
        Message::DisplayWrotePrefix => "기록함",
        Message::DisplayEditedPrefix => "편집함",
        Message::DisplayListedPrefix => "목록 조회함",
        Message::EntriesUnit => "개 항목",
        Message::MatchesUnit => "개 일치",
        Message::ReachedMaxIterationsPrefix => "\n\n최대 반복 횟수(",
        Message::ReachedMaxIterationsSuffix => ")에 도달하여 현재 결과를 반환합니다.",
        Message::InteractiveQueuedRequest => "  대기 중인 요청:",
        Message::InteractiveRunning => "  실행 중...",
        Message::InteractiveReadyForNextRequest => "  다음 요청을 받을 준비가 되었습니다",
        Message::InteractiveModelLabel => "모델",
        Message::InteractiveParamsLabel => "매개변수",
        Message::InteractiveWorkingDirLabel => "작업 디렉터리",
        Message::InteractiveWorkspaceLabel => "작업 공간",
        Message::InteractiveProviderLabel => "공급자",
        Message::InteractiveProviderLocal => "로컬 설정",
        Message::InteractiveModeFieldLabel => "모드",
        Message::InteractiveNetworkFieldLabel => "네트워크",
        Message::InteractiveEndpointLabel => "엔드포인트",
        Message::InteractiveAvailableModels => "사용 가능한 모델:",
        Message::InteractiveModelUsageHint => "사용법: /model <name>",
        Message::InteractiveAvailableCommands => "사용 가능한 명령:",
        Message::InteractiveSlashHelpHint => "도움말 표시",
        Message::InteractiveSlashModelHint => "모델 목록 보기, /model <name> 으로 전환",
        Message::InteractiveSlashExecutorHint => "코어 실행 단위 나열 또는 전환",
        Message::InteractiveAvailableExecutors => "실행 단위:",
        Message::InteractiveExecutorUsage => "사용법: /executor <nola|codex|claude_code|opencode|antigravity>",
        Message::InteractiveExecutorSelected => "선택한 실행 단위:",
        Message::InteractiveExecutorUnavailable => "사용할 수 없는 실행 단위:",
        Message::InteractiveExecutorLabel => "실행기",
        Message::InteractiveSlashDebugHint => "디버그 세부 정보 표시 또는 전환",
        Message::InteractiveSlashDebugUsage => "사용법: /debug on | /debug off",
        Message::InteractiveSlashAllowHint => "권한 모드 설정",
        Message::InteractiveSlashAllowUsage => "사용법: /allow — 권한 선택 패널 열기",
        Message::InteractiveAllowPanelTitle => "권한 모드",
        Message::InteractiveAllowPanelCautious => "신중",
        Message::InteractiveAllowPanelCautiousDesc => "삭제, 덮어쓰기, 민감한 데이터 접근, 외부 전송 및 새 연결은 먼저 확인하며, 일반 읽기와 새 파일 생성은 바로 진행합니다.",
        Message::InteractiveAllowPanelAllowAll => "모두 허용",
        Message::InteractiveAllowPanelAllowAllDesc => "이 세션에서는 Nola 승인 프롬프트가 표시되지 않습니다. 시스템 권한은 계속 적용됩니다.",
        Message::InteractiveAllowPanelSelected => "선택됨:",
        Message::InteractiveAllowPanelCancelled => "취소됨",
        Message::InteractiveSlashGoalHint => "목표를 향해 자율적으로 작업",
        Message::InteractiveGoalControlPending => "목표 제어 작업이 대기열에 추가되었습니다. 적용 상태는 /goal status 로 확인하세요.",
        Message::InteractiveGoalAlreadyActive => "이미 활성 목표가 있습니다. /goal resume 를 사용하거나 /goal off 로 종료한 뒤 새로 시작하세요.",
        Message::InteractiveSlashGoalUsage => "사용법: /goal [start] <objective> [-c/--constraints <constraints>] | /goal status | /goal pause | /goal resume | /goal off",
        Message::InteractiveSlashInitHint => "NOLA.md 프로젝트 메모리 생성 (--dry-run, --force, --output PATH)",
        Message::InteractiveSlashCdHint => "작업 공간을 전환하고 새 프로젝트 세션 시작 (/cd - 로 이전 위치로 복귀)",
        Message::InteractiveSlashCompactHint => "전체 재생 기록을 유지하면서 모델 컨텍스트 압축",
        Message::InteractiveSlashNewHint => "현재 작업 공간에서 새 세션 시작",
        Message::InteractiveSlashPlanHint => "현재 세션의 실행 계획 표시",
        Message::InteractiveSlashJobsHint => "지속된 백그라운드 작업 목록 보기, 검사 또는 중지",
        Message::InteractiveSlashResumeHint => "세션 복원 페이지 열기",
        Message::InteractiveSlashContinueHint => "가장 최근 프로젝트 세션 계속하기",
        Message::InteractiveSlashHistoryHint => "전체 세션 기록 다시 출력",
        Message::InteractiveSlashStatusHint => "현재 런타임 상태 표시",
        Message::InteractiveSlashExitHint => "현재 세션 종료",
        Message::InteractiveSlashQuitHint => "q/quit/exit 로도 종료 가능",
        Message::InteractiveWelcomeTagline => "로컬 우선 개인 어시스턴트",
        Message::InteractiveWelcomeHint => "첫 메시지를 입력하세요 · 명령어 목록은 /help",
        Message::InteractiveAutoValue => "자동",
        Message::InteractiveNetworkAllowValue => "허용",
        Message::InteractiveNetworkAskValue => "확인",
        Message::InteractiveNetworkDenyValue => "차단",
        Message::InteractiveContextLabel => "컨텍스트",
        Message::InteractiveTokensLabel => "토큰",
        Message::InteractiveTokensUnit => "토큰",
        Message::InteractiveMessagesUnit => "개 메시지",
        Message::InteractiveAllowPanelErrorPrefix => "권한 패널 오류:",
        Message::InteractiveGoalStatusPrefix => "목표:",
        Message::InteractiveGoalStatePrefix => "상태:",
        Message::InteractiveGoalStateRunning => "실행 중",
        Message::InteractiveGoalStatePaused => "일시 중지됨",
        Message::InteractiveGoalStateNeedsUser => "사용자 대기 중",
        Message::InteractiveGoalStateFailed => "실패",
        Message::InteractiveGoalResumeStarted => "목표 재개 중:",
        Message::InteractiveGoalNoResumable => "이 세션에는 재개할 수 있는 목표가 없습니다.",
        Message::InteractiveWorkspaceStatusPrefix => "작업 공간:",
        Message::InteractiveCdUsage => "사용법: /cd PATH · /cd - 이전 작업 공간으로 돌아가기",
        Message::InteractiveCompactTitle => "컨텍스트 압축",
        Message::InteractiveCompactNoActiveSession => "활성 세션 없음",
        Message::InteractiveCompactAlready => "이미 압축됨",
        Message::InteractiveCompactCompleted => "압축 완료",
        Message::InteractiveCompactSummarized => "요약됨",
        Message::InteractiveCompactKeptRecent => "최근 항목 유지",
        Message::InteractiveCompactSaved => "절감됨",
        Message::InteractiveCompactHistoryPreserved => "재생을 위해 전체 대화 기록이 보존됩니다",
        Message::InteractiveCompactFailedPrefix => "컨텍스트 압축 실패:",
        Message::InteractiveNewSessionTitle => "새 세션",
        Message::InteractiveNewSessionPending => "다음 메시지부터 새 세션이 시작됩니다",
        Message::InteractiveJobsTitle => "백그라운드 작업",
        Message::InteractiveJobsUsage => "사용법: /jobs · /jobs ID · /jobs status ID · /jobs stop ID",
        Message::InteractiveUnknownCommandHint => "알 수 없는 명령입니다. 사용 가능한 명령을 보려면 /help 를 입력하세요",
        Message::InteractiveChoiceLabel => "선택해 주세요",
        Message::InteractiveModelStatusPrefix => "모델:",
        Message::InteractiveDebugStatusPrefix => "디버그:",
        Message::InteractiveAllowStatusPrefix => "권한:",
        Message::InteractiveDefaultModelLabel => "(기본값)",
        Message::InteractiveTurnsLabel => "턴",
        Message::InteractiveToolsLabel => "도구",
        Message::InteractiveNetworkLabel => "네트워크",
        Message::InteractiveInputPlaceholder => "메시지를 입력하세요…",
        Message::InteractiveExecutionDraftPrefix => "↳",
        Message::InteractiveExecutionQueuedPrefix => "대기 중인 후속 지시:",
        Message::InteractiveExecutionInterruptHint => "Esc 를 눌러 현재 실행 중단",
        Message::InteractiveExecutionInputHint => "Esc 중단 · Enter 후속 지시 대기열 추가",
        Message::AgentStatusThinking => "생각 중",
        Message::AgentStatusRetryingModel => "모델 응답 재시도 중",
        Message::AgentStatusStrictToolStrategy => "엄격한 도구 전략",
        Message::AgentStatusChooseFromAvailableTools => "사용 가능한 도구에서 선택",
        Message::AgentStatusReadyToContinue => "계속할 준비 완료",
        Message::AgentStatusEnteredMode => "모드 진입",
        Message::AgentStatusReplyingDirectly => "직접 응답 중",
        Message::AgentStatusExecutingPlan => "계획 실행 중",
        Message::AgentStatusWaitingForUserConfirmation => "사용자 확인 대기 중",
        Message::AgentStatusProcessing => "처리 중",
        Message::AgentStatusPreparing => "준비 중",
        Message::AgentStatusModelUnavailable => "모델을 일시적으로 사용할 수 없음",
        Message::AgentDoneLabel => "완료",
        Message::AgentErrorLabel => "오류",
        Message::AgentToolLabel => "도구",
        Message::AgentAgentLabel => "에이전트",
        Message::AgentSessionLabel => "세션",
        Message::AgentStatusLabel => "상태",
        Message::AgentInfoLabel => "정보",
        Message::AgentReadyCurrentModePrefix => "준비됨, 현재 모드:",
        Message::AgentReadyCurrentModeSuffix => "",
        Message::AgentStatusDetailsSeparator => ", ",
        Message::AgentPreparingToExecutePrefix => "실행 준비 중:",
        Message::AgentActionCountSuffixSingle => "개 작업",
        Message::AgentActionCountSuffixPlural => "개 작업",
        Message::AgentTurnPrefix => "턴",
        Message::AgentTurnSuffix => "",
        Message::AgentInspectingProject => "프로젝트 검사 중",
        Message::AgentEditingFiles => "파일 편집 중",
        Message::AgentRunningCommand => "명령 실행 중",
        Message::AgentAccessingNetwork => "네트워크 접근 중",
        Message::AgentSearchingTools => "도구 검색 중",
        Message::AgentStartingSubtask => "하위 작업 시작 중",
        Message::AgentFileModified => "파일 수정됨",
        Message::AgentCommandCompleted => "명령 완료됨",
        Message::AgentNetworkRequestCompleted => "네트워크 요청 완료됨",
        Message::AgentSubtaskCompleted => "하위 작업 완료됨",
        Message::AgentFailedToModifyFile => "파일 수정 실패",
        Message::AgentCommandFailed => "명령 실행 실패",
        Message::AgentNetworkRequestFailed => "네트워크 요청 실패",
        Message::AgentSubtaskFailed => "하위 작업 실패",
        Message::AgentOperationFailed => "작업 실패",
        Message::InteractiveUnknownPath => "(알 수 없음)",
        Message::InteractiveStatusOn => "켜짐",
        Message::InteractiveStatusOff => "꺼짐",
        Message::InteractiveInvalidLanguageValue => "지원되지 않는 언어",
        Message::InvalidPortValue => "--port 는 1 에서 65535 사이의 숫자여야 합니다",
        Message::UnknownAgentModePrefix => "알 수 없는 --agent-mode",
        Message::NetworkPolicyMustBeOneOf => "--network-policy 는 allow, ask, deny 중 하나여야 합니다",
        Message::ForceToolSetMustBeOneOf => "--force-tool-set 은 auto, all, write 중 하나여야 합니다",
        Message::FailedReadKeyEventPrefix => "키 이벤트 읽기 실패:",
        Message::FailedFlushPromptPrefix => "프롬프트 플러시 실패:",
        Message::InteractiveRawModeFailedPrefix => "로우 입력 모드 활성화 실패:",
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

#[cfg(test)]
mod cli_i18n_helper_tests {
    use super::*;

    #[test]
    fn shared_cli_i18n_helpers_cover_all_nine_locales() {
        let _guard = crate::store::test_env_lock();
        let previous = current_language();
        let locales = [
            Language::ZhHans,
            Language::ZhHant,
            Language::En,
            Language::De,
            Language::Es,
            Language::Fr,
            Language::Ja,
            Language::Ko,
            Language::Ru,
        ];
        for lang in locales {
            set_language(lang);
            assert!(!cli_stopping_to_exit().is_empty());
            assert!(!cli_stopping_to_resume().is_empty());
            assert!(cli_view_state_restore_failed("disk full").contains("disk full"));
            assert!(!cli_command_unavailable_in_context().is_empty());
            assert!(cli_pasted_content_badge(42).contains("42"));
            assert!(!cli_history_empty().is_empty());
            for key in [
                "last response",
                "continue",
                "state",
                "init",
                "wiki",
                "plan",
                "plan history",
                "plan diff",
                "skills",
                "mcp",
                "guide",
            ] {
                assert!(!cli_section_label(key).is_empty());
            }
            assert_eq!(cli_section_label("custom_unknown"), "custom_unknown");
            for status in [
                "available",
                "not installed",
                "not authenticated",
                "incompatible",
                "unreachable",
            ] {
                assert!(!cli_executor_status_label(status).is_empty());
            }
            assert_eq!(cli_executor_status_label("custom_status"), "custom_status");
        }
        set_language(Language::ZhHans);
        assert_eq!(cli_section_label("plan history"), "计划历史");
        assert_eq!(cli_executor_status_label("not installed"), "未安装");
        set_language(Language::En);
        assert_eq!(cli_section_label("plan history"), "plan history");
        assert_eq!(cli_executor_status_label("not installed"), "not installed");
        set_language(previous);
    }
}

include!("i18n_tool_errors.rs");
