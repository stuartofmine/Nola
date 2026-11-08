#!/usr/bin/env ruby
# SPDX-License-Identifier: Apache-2.0
# Synchronize Mac source localization keys without pretending that untranslated
# English fallbacks are finished translations.

require "json"
require "cgi"

ROOT = File.expand_path("..", __dir__)
CATALOG_PATH = File.join(ROOT, "NolaMac", "Localizable.xcstrings")
LOCALES = %w[en de es fr ja ko ru zh-Hans zh-Hant].freeze

# String(localized:) calls without a default value expose their key as the
# English copy unless the catalog already contains a source value. Keep the
# source language human-readable before translation memory is applied.
KEY_DEFAULTS = {
  "mac.command.iphone_health" => "iPhone Health",
  "mac.command.menu.conversation" => "Conversation",
  "mac.command.new_conversation" => "New Conversation",
  "mac.command.web_sources" => "Web Sources",
  "mac.window.conversation" => "Conversation",
  "mac.window.iphone_health" => "iPhone Health",
  "mac.window.web_sources" => "Web Sources",
  "mac.web_sources.save" => "Save",
}.freeze

# Reviewed product vocabulary that is shared by the Mac navigation, inspector,
# permissions, browser, model and diagnostic surfaces. This is deliberately
# kept in the synchronizer so newly introduced duplicate copy does not silently
# fall back to English again.
CURATED_TRANSLATIONS = {
  "de" => {
    "Agent Host protocol error" => "Agent-Host-Protokollfehler", "Allow this operation?" => "Diesen Vorgang erlauben?",
    "Available in This Conversation" => "In dieser Unterhaltung verfügbar", "Browser" => "Browser", "Cancel" => "Abbrechen",
    "Conversation" => "Unterhaltung", "Conversation Details" => "Unterhaltungsdetails", "Data Sources" => "Datenquellen",
    "Delete" => "Löschen", "Download" => "Herunterladen", "Files" => "Dateien", "Knowledge" => "Wissen",
    "Live web search" => "Live-Websuche", "Local agent" => "Lokaler Agent", "Local Models" => "Lokale Modelle",
    "Local model ready" => "Lokales Modell bereit", "Logs Cleared" => "Protokolle gelöscht", "Manage" => "Verwalten",
    "New Conversation" => "Neue Unterhaltung", "Refresh" => "Aktualisieren", "Response" => "Antwort",
    "Resume" => "Fortsetzen", "Runtime" => "Laufzeit", "Save memory" => "Erinnerung speichern",
    "Search memory" => "Erinnerungen durchsuchen", "Search the web" => "Web durchsuchen", "Settings" => "Einstellungen",
    "Thinking" => "Denkt nach", "Web Sources" => "Webquellen", "Working Context" => "Arbeitskontext",
    "iPhone Health" => "iPhone-Gesundheitsdaten", "Nola could not start the local agent: %@" => "Nola konnte den lokalen Agenten nicht starten: %@",
    "The local agent is already running." => "Der lokale Agent wird bereits ausgeführt.", "The local agent could not be started." => "Der lokale Agent konnte nicht gestartet werden."
  },
  "es" => {
    "Agent Host protocol error" => "Error de protocolo de Agent Host", "Allow this operation?" => "¿Permitir esta operación?",
    "Available in This Conversation" => "Disponible en esta conversación", "Browser" => "Navegador", "Cancel" => "Cancelar",
    "Conversation" => "Conversación", "Conversation Details" => "Detalles de la conversación", "Data Sources" => "Fuentes de datos",
    "Delete" => "Eliminar", "Download" => "Descargar", "Files" => "Archivos", "Knowledge" => "Conocimiento",
    "Live web search" => "Búsqueda web en vivo", "Local agent" => "Agente local", "Local Models" => "Modelos locales",
    "Local model ready" => "Modelo local listo", "Logs Cleared" => "Registros borrados", "Manage" => "Administrar",
    "New Conversation" => "Nueva conversación", "Refresh" => "Actualizar", "Response" => "Respuesta",
    "Resume" => "Reanudar", "Runtime" => "Ejecución", "Save memory" => "Guardar memoria",
    "Search memory" => "Buscar en la memoria", "Search the web" => "Buscar en la web", "Settings" => "Ajustes",
    "Thinking" => "Pensando", "Web Sources" => "Fuentes web", "Working Context" => "Contexto de trabajo",
    "iPhone Health" => "Salud del iPhone", "Nola could not start the local agent: %@" => "Nola no pudo iniciar el agente local: %@",
    "The local agent is already running." => "El agente local ya está en ejecución.", "The local agent could not be started." => "No se pudo iniciar el agente local."
  },
  "fr" => {
    "Agent Host protocol error" => "Erreur de protocole de l’hôte Agent", "Allow this operation?" => "Autoriser cette opération ?",
    "Available in This Conversation" => "Disponible dans cette conversation", "Browser" => "Navigateur", "Cancel" => "Annuler",
    "Conversation" => "Conversation", "Conversation Details" => "Détails de la conversation", "Data Sources" => "Sources de données",
    "Delete" => "Supprimer", "Download" => "Télécharger", "Files" => "Fichiers", "Knowledge" => "Connaissances",
    "Live web search" => "Recherche web en direct", "Local agent" => "Agent local", "Local Models" => "Modèles locaux",
    "Local model ready" => "Modèle local prêt", "Logs Cleared" => "Journaux effacés", "Manage" => "Gérer",
    "New Conversation" => "Nouvelle conversation", "Refresh" => "Actualiser", "Response" => "Réponse",
    "Resume" => "Reprendre", "Runtime" => "Exécution", "Save memory" => "Enregistrer la mémoire",
    "Search memory" => "Rechercher dans la mémoire", "Search the web" => "Rechercher sur le web", "Settings" => "Réglages",
    "Thinking" => "Réflexion en cours", "Web Sources" => "Sources web", "Working Context" => "Contexte de travail",
    "iPhone Health" => "Santé de l’iPhone", "Nola could not start the local agent: %@" => "Nola n’a pas pu démarrer l’agent local : %@",
    "The local agent is already running." => "L’agent local est déjà en cours d’exécution.", "The local agent could not be started." => "L’agent local n’a pas pu être démarré."
  },
  "ja" => {
    "Agent Host protocol error" => "Agent Host プロトコルエラー", "Allow this operation?" => "この操作を許可しますか？",
    "Available in This Conversation" => "この会話で利用可能", "Browser" => "ブラウザ", "Cancel" => "キャンセル",
    "Conversation" => "会話", "Conversation Details" => "会話の詳細", "Data Sources" => "データソース",
    "Delete" => "削除", "Download" => "ダウンロード", "Files" => "ファイル", "Knowledge" => "ナレッジ",
    "Live web search" => "リアルタイム Web 検索", "Local agent" => "ローカルエージェント", "Local Models" => "ローカルモデル",
    "Local model ready" => "ローカルモデルの準備完了", "Logs Cleared" => "ログを消去しました", "Manage" => "管理",
    "New Conversation" => "新しい会話", "Refresh" => "更新", "Response" => "回答",
    "Resume" => "再開", "Runtime" => "ランタイム", "Save memory" => "メモリを保存",
    "Search memory" => "メモリを検索", "Search the web" => "Web を検索", "Settings" => "設定",
    "Thinking" => "思考中", "Web Sources" => "Web ソース", "Working Context" => "作業コンテキスト",
    "iPhone Health" => "iPhone ヘルスケア", "Nola could not start the local agent: %@" => "Nola はローカルエージェントを起動できませんでした：%@",
    "The local agent is already running." => "ローカルエージェントはすでに実行中です。", "The local agent could not be started." => "ローカルエージェントを起動できませんでした。"
  },
  "ko" => {
    "Agent Host protocol error" => "Agent Host 프로토콜 오류", "Allow this operation?" => "이 작업을 허용할까요?",
    "Available in This Conversation" => "이 대화에서 사용 가능", "Browser" => "브라우저", "Cancel" => "취소",
    "Conversation" => "대화", "Conversation Details" => "대화 세부 정보", "Data Sources" => "데이터 소스",
    "Delete" => "삭제", "Download" => "다운로드", "Files" => "파일", "Knowledge" => "지식",
    "Live web search" => "실시간 웹 검색", "Local agent" => "로컬 에이전트", "Local Models" => "로컬 모델",
    "Local model ready" => "로컬 모델 준비 완료", "Logs Cleared" => "로그 지움", "Manage" => "관리",
    "New Conversation" => "새 대화", "Refresh" => "새로 고침", "Response" => "응답",
    "Resume" => "재개", "Runtime" => "런타임", "Save memory" => "메모리 저장",
    "Search memory" => "메모리 검색", "Search the web" => "웹 검색", "Settings" => "설정",
    "Thinking" => "생각 중", "Web Sources" => "웹 소스", "Working Context" => "작업 컨텍스트",
    "iPhone Health" => "iPhone 건강", "Nola could not start the local agent: %@" => "Nola가 로컬 에이전트를 시작하지 못했습니다: %@",
    "The local agent is already running." => "로컬 에이전트가 이미 실행 중입니다.", "The local agent could not be started." => "로컬 에이전트를 시작할 수 없습니다."
  },
  "ru" => {
    "Agent Host protocol error" => "Ошибка протокола Agent Host", "Allow this operation?" => "Разрешить эту операцию?",
    "Available in This Conversation" => "Доступно в этом разговоре", "Browser" => "Браузер", "Cancel" => "Отмена",
    "Conversation" => "Разговор", "Conversation Details" => "Сведения о разговоре", "Data Sources" => "Источники данных",
    "Delete" => "Удалить", "Download" => "Скачать", "Files" => "Файлы", "Knowledge" => "Знания",
    "Live web search" => "Поиск в интернете", "Local agent" => "Локальный агент", "Local Models" => "Локальные модели",
    "Local model ready" => "Локальная модель готова", "Logs Cleared" => "Журналы очищены", "Manage" => "Управление",
    "New Conversation" => "Новый разговор", "Refresh" => "Обновить", "Response" => "Ответ",
    "Resume" => "Продолжить", "Runtime" => "Среда выполнения", "Save memory" => "Сохранить память",
    "Search memory" => "Поиск в памяти", "Search the web" => "Искать в интернете", "Settings" => "Настройки",
    "Thinking" => "Размышление", "Web Sources" => "Веб-источники", "Working Context" => "Рабочий контекст",
    "iPhone Health" => "Здоровье iPhone", "Nola could not start the local agent: %@" => "Nola не удалось запустить локального агента: %@",
    "The local agent is already running." => "Локальный агент уже запущен.", "The local agent could not be started." => "Не удалось запустить локального агента."
  },
}.freeze

SECONDARY_PAGE_KO = {
  "Data Center" => "데이터 센터",
  "Control what Nola can perceive and how conversations may use it." => "Nola가 인식할 수 있는 정보와 대화에서 사용하는 방식을 관리합니다.",
  "Source Health" => "데이터 소스 상태",
  "All connected sources passed a real read test." => "연결된 모든 소스가 실제 읽기 테스트를 통과했습니다.",
  "%d sources need attention" => "주의가 필요한 소스 %d개",
  "Every Conversation" => "모든 대화",
  "Per Conversation" => "대화별",
  "Ask When Relevant" => "필요할 때 묻기",
  "Blocked" => "차단됨",
  "Verified" => "확인됨",
  "Validation failed" => "검증 실패",
  "Reader" => "읽기 방식",
  "Data Format" => "데이터 형식",
  "Redacted Sample" => "민감 정보 제거 샘플",
  "Test Connection" => "연결 테스트",
  "Never Tested" => "테스트하지 않음",
  "No validated contract. Test the connection before using this source." => "검증된 계약이 없습니다. 이 소스를 사용하기 전에 연결을 테스트하세요.",
  "Recent Usage" => "최근 사용",
  "No source usage yet" => "아직 데이터 소스 사용 기록이 없습니다",
  "Usage appears after a conversation or automation reads a source." => "대화 또는 자동화가 소스를 읽으면 사용 기록이 표시됩니다.",
  "Web Sources" => "웹 소스",
  "Sources available to retrieval context" => "검색 컨텍스트에서 사용할 수 있는 소스",
  "Add Source" => "소스 추가",
  "No Web Sources" => "웹 소스 없음",
  "Add JSON, RSS, Atom, or text sources for retrieval context." => "검색 컨텍스트에 JSON, RSS, Atom 또는 텍스트 소스를 추가하세요.",
  "Select a source to review its cached content and refresh status." => "소스를 선택하여 캐시된 내용과 새로 고침 상태를 확인하세요.",
  "Web Source Error" => "웹 소스 오류",
  "New Web Source" => "새 웹 소스",
  "Edit Web Source" => "웹 소스 편집",
  "Testing connection…" => "연결 테스트 중…",
  "Connection verified" => "연결 확인됨",
  "The source response exceeds the 2 MB limit." => "소스 응답이 2MB 제한을 초과합니다.",
  "The source returned no readable content." => "소스가 읽을 수 있는 콘텐츠를 반환하지 않았습니다.",
  "No Cached Content" => "캐시된 콘텐츠 없음",
  "Refresh this source to fetch its current content." => "현재 콘텐츠를 가져오려면 이 소스를 새로 고치세요.",
  "Run History" => "실행 기록",
  "Synced from %@ · %@" => "%@에서 동기화 · %@",
  "iCloud Health sync requires a signed build with the KVS capability" => "iCloud 건강 동기화에는 KVS 기능이 포함된 서명된 빌드가 필요합니다",
  "Waiting for an aggregate Health snapshot from Nola on iPhone" => "iPhone의 Nola에서 집계된 건강 스냅샷을 기다리는 중",
  "Sync Now" => "지금 동기화",
  "Active Energy" => "활동 에너지",
  "Resting Heart Rate" => "안정 시 심박수",
  "SEVEN-DAY STEPS" => "7일 걸음 수",
  "No Synced Health Summary" => "동기화된 건강 요약 없음",
  "iCloud Sync Unavailable" => "iCloud 동기화를 사용할 수 없음",
  "Open Nola’s Health dashboard on iPhone after granting Health access. The next successful refresh publishes an aggregate snapshot through iCloud." => "건강 접근 권한을 허용한 뒤 iPhone에서 Nola 건강 대시보드를 여세요. 다음 새로 고침이 성공하면 집계 스냅샷이 iCloud를 통해 게시됩니다.",
  "Run a signed Nola build with the iCloud key-value store capability to receive Health summaries from iPhone." => "iPhone 건강 요약을 받으려면 iCloud 키-값 저장소 기능이 포함된 서명된 Nola 빌드를 실행하세요.",
  "May be outdated" => "오래된 정보일 수 있음",
  "Up to date" => "최신 상태",
  "Automations" => "자동화",
  "Automation Count" => "자동화 수",
  "Recurring work, explicit workspaces, and visible run history" => "반복 작업, 명확한 작업 공간 및 확인 가능한 실행 기록",
  "New Automation" => "새 자동화",
  "Edit Automation" => "자동화 편집",
  "No Automations" => "자동화 없음",
  "Schedule a prompt to run daily or weekly in a selected workspace." => "선택한 작업 공간에서 프롬프트를 매일 또는 매주 실행하도록 예약하세요.",
  "Run Now" => "지금 실행",
  "View Run History" => "실행 기록 보기",
  "Reveal Log in Finder" => "Finder에서 로그 보기",
  "Agent prompt" => "에이전트 프롬프트",
  "Automation Error" => "자동화 오류",
  "Activity" => "활동",
  "Agent and automation results with local run details" => "로컬 실행 세부 정보가 포함된 에이전트 및 자동화 결과",
  "Search activity" => "활동 검색",
  "Activity Center" => "활동 센터",
  "Agent and automation results stay available here." => "에이전트 및 자동화 결과가 여기에 보관됩니다.",
  "Open Conversation" => "대화 열기",
  "Clear History" => "기록 지우기",
  "Clear Activity History?" => "활동 기록을 지울까요?",
  "This removes the local history list. Conversation and automation logs are not deleted." => "로컬 기록 목록만 제거됩니다. 대화 및 자동화 로그는 삭제되지 않습니다.",
  "Started" => "시작됨",
  "Duration" => "소요 시간",
  "Result" => "결과",
  "OUTPUT" => "출력",
  "%lld sec" => "%lld초",
  "%lld min %lld sec" => "%lld분 %lld초",
  "Local Capabilities" => "로컬 기능",
  "System access and tools available to Nola on this Mac" => "이 Mac에서 Nola가 사용할 수 있는 시스템 접근 권한과 도구",
  "Browse, configure, and edit tools available to this Mac." => "이 Mac에서 사용할 수 있는 도구를 찾아보고 구성하거나 편집합니다.",
  "Chrome Extension" => "Chrome 확장 프로그램",
  "Ready to connect" => "연결 준비 완료",
  "Data Management" => "데이터 관리",
  "Back up or reset data owned by Nola for Mac" => "Mac용 Nola 데이터 백업 또는 초기화",
  "Backup and Restore" => "백업 및 복원",
  "Export Backup" => "백업 내보내기",
  "Restore Backup" => "백업 복원",
  "Clear Application Data…" => "앱 데이터 지우기…",
  "Data Management Error" => "데이터 관리 오류",
  "Runtime Diagnostics" => "런타임 진단",
  "Runtime status, logs, and support export." => "런타임 상태, 로그 및 지원용 내보내기입니다.",
  "Detailed Logging" => "상세 로깅",
  "Warnings & Errors" => "경고 및 오류",
  "Search logs" => "로그 검색",
  "No log entries match current filter" => "현재 필터와 일치하는 로그가 없습니다",
  "Waiting for log entries" => "로그 항목을 기다리는 중",
  "Detailed logging is off." => "상세 로깅이 꺼져 있습니다.",
  "Prompt Library" => "프롬프트 라이브러리",
  "Create reusable instructions without adding controls to the composer." => "입력창에 컨트롤을 추가하지 않고 재사용 가능한 지침을 만듭니다.",
  "Search templates" => "템플릿 검색",
  "Delete Prompt Template?" => "프롬프트 템플릿을 삭제할까요?",
  "This removes the custom template from this Mac. This action cannot be undone." => "이 Mac에서 사용자 지정 템플릿이 제거됩니다. 이 작업은 취소할 수 없습니다.",
  "%d tokens" => "%d 토큰",
  "Discard unsaved changes?" => "저장하지 않은 변경 사항을 버릴까요?",
  "Discard Changes" => "변경 사항 버리기",
  "Keep Editing" => "계속 편집",
  "Your prompt template has not been saved." => "프롬프트 템플릿이 저장되지 않았습니다.",
  "Local Models" => "로컬 모델",
  "Manage the shared Apple-client MLX model cache" => "Apple 클라이언트가 공유하는 MLX 모델 캐시 관리",
  "Delete Local Model?" => "로컬 모델을 삭제할까요?",
  "The downloaded model files will be removed from this Mac." => "다운로드한 모델 파일이 이 Mac에서 제거됩니다.",
  "Download Source" => "다운로드 소스",
  "Estimated download · %@" => "예상 다운로드 · %@",
  "SELECTED" => "선택됨",
  "Use for Chat" => "채팅에 사용",
  "Local model storage" => "로컬 모델 저장 공간",
  "Model Center" => "모델 센터",
  "Models stay scoped to their execution environment" => "모델은 각 실행 환경에 맞게 구분됩니다",
  "Manage Local Models…" => "로컬 모델 관리…",
  "Connected Host" => "연결된 호스트",
  "Chat Only" => "채팅 전용",
  "Available model" => "사용 가능한 모델",
}.freeze

RUNTIME_ZH_HANS = {
  "Nola could not start the local agent: %@" => "Nola 无法启动本地智能体：%@",
  "The local agent is already running." => "本地智能体已在运行。",
  "The local agent could not be started." => "无法启动本地智能体。",
  "The local agent exited with status %d." => "本地智能体已退出，状态码为 %d。",
  "The local agent finished without a terminal event." => "本地智能体结束时未返回终止事件。",
  "The local agent was terminated by macOS (signal %d). Rebuild the Mac app so its embedded Agent Host is signed correctly." => "本地智能体被 macOS 终止（信号 %d）。请重新构建 Mac App，确保内置 Agent Host 已正确签名。",
}.freeze

def merge_curated_memory!(memory)
  CURATED_TRANSLATIONS.each do |locale, translations|
    translations.each { |english, value| memory[locale][english] = value }
  end
  SECONDARY_PAGE_KO.each { |english, value| memory["ko"][english] = value }
  RUNTIME_ZH_HANS.each { |english, value| memory["zh-Hans"][english] = value }
end

def source_pairs
  pairs = {}
  Dir[File.join(ROOT, "NolaMac", "**", "*.swift")].each do |path|
    source = File.read(path)
    source.scan(/\b\w+Text\(\s*"([^"]+)"\s*,\s*"((?:\\.|[^"])*)"/m) do |key, fallback|
      pairs[key] = fallback.gsub('\\"', '"')
    end
    source.scan(/NSLocalizedString\(\s*"([^"]+)"\s*,\s*value:\s*"((?:\\.|[^"])*)"/m) do |key, fallback|
      pairs[key] = fallback.gsub('\\"', '"')
    end
    source.scan(/String\(localized:\s*"((?:\\.|[^"])*)"/m) do |value|
      pairs[value.first] = value.first
    end
  end
  pairs
end

def existing_translation_memory(catalog)
  memory = Hash.new { |hash, locale| hash[locale] = {} }
  catalog.fetch("strings", {}).each_value do |entry|
    localizations = entry.fetch("localizations", {})
    english = localizations.dig("en", "stringUnit", "value")
    next if english.nil? || english.empty?
    localizations.each do |locale, localization|
      value = localization.dig("stringUnit", "value")
      memory[locale][english] = value if value && !value.empty? && value != english
    end
  end
  memory
end

def parse_apple_strings(path)
  return {} unless File.file?(path)
  File.read(path).scan(/^\s*"((?:\\.|[^"])*)"\s*=\s*"((?:\\.|[^"])*)"\s*;/)
      .to_h
      .transform_values { |value| value.gsub('\\"', '"').gsub('\\n', "\n") }
end

def parse_android_strings(path)
  return {} unless File.file?(path)
  File.read(path).scan(/<string\s+name="([^"]+)"[^>]*>(.*?)<\/string>/m)
      .to_h
      .transform_values { |value| CGI.unescapeHTML(value.gsub(/<[^>]+>/, "")).strip }
end

def merge_cross_platform_memory!(memory)
  apple_en = parse_apple_strings(File.join(ROOT, "Nola", "en.lproj", "Localizable.strings"))
  {
    "de" => "de", "es" => "es", "fr" => "fr", "ja" => "ja",
    "ru" => "ru", "zh-Hans" => "zh-Hans",
  }.each do |locale, directory|
    localized = parse_apple_strings(File.join(ROOT, "Nola", "#{directory}.lproj", "Localizable.strings"))
    apple_en.each do |key, english|
      value = localized[key]
      memory[locale][english] = value if value && !value.empty? && value != english
    end
  end

  android_en = parse_android_strings(File.join(ROOT, "android", "app", "src", "main", "res", "values", "strings.xml"))
  {
    "de" => "values-de", "es" => "values-es", "fr" => "values-fr", "ja" => "values-ja",
    "ru" => "values-ru", "zh-Hans" => "values-zh",
  }.each do |locale, directory|
    localized = parse_android_strings(File.join(ROOT, "android", "app", "src", "main", "res", directory, "strings.xml"))
    android_en.each do |key, english|
      value = localized[key]
      memory[locale][english] = value if value && !value.empty? && value != english
    end
  end
end

ZH_HANS = {
  "API models" => "API 模型", "Browser" => "浏览器", "Computer use" => "电脑控制",
  "Conversation Recall" => "会话回忆", "Conversation Wiki" => "会话 Wiki",
  "Conversation message" => "会话消息", "Estimated download" => "预计下载大小",
  "Exported" => "已导出", "Knowledge" => "知识", "Knowledge Points" => "知识要点",
  "Live web search" => "实时联网搜索", "Live web search failed." => "实时联网搜索失败。",
  "Local agent" => "本地智能体", "Local models" => "本地模型", "Logs Cleared" => "日志已清除",
  "Native MLX inference" => "原生 MLX 推理", "Not downloaded" => "未下载",
  "OpenAI-compatible provider" => "OpenAI 兼容服务", "Paused" => "已暂停",
  "Plan and execute" => "规划并执行", "Points" => "要点", "Read" => "读取",
  "Recall" => "回忆", "Reset Filters" => "重置筛选", "Response" => "回复",
  "Review a project" => "检查项目", "Saving…" => "正在保存…", "Select" => "选择",
  "Selected" => "已选择", "Shell and Git" => "Shell 与 Git", "Skill" => "技能",
  "Start" => "启动", "Starting" => "正在启动", "Status" => "状态", "Wiki" => "Wiki",
  "Work with a document" => "处理文档", "Working Context" => "工作上下文",
  "Your message" => "你的消息", "Cancel" => "取消", "Dismiss" => "关闭", "OK" => "好",
  "Refresh" => "刷新", "Working" => "处理中", "Reset" => "重置",
  "Backup and Restore" => "备份与恢复", "Data Management" => "数据管理",
  "Export Backup" => "导出备份", "Restore Backup" => "恢复备份", "Restore" => "恢复",
  "Clear Application Data…" => "清除应用数据…", "Clear Application Data?" => "清除应用数据？",
  "Clear" => "清除", "Data Management Error" => "数据管理错误", "Backup exported" => "备份已导出",
  "Activity" => "活动", "Automations" => "自动化", "Conversations" => "会话",
  "Local Data" => "本地数据", "Notifications" => "通知", "Web Sources" => "网页数据源",
  "Active Energy" => "活动能量", "Day" => "日期", "Data Center" => "数据中心",
  "Distance" => "距离", "Exercise" => "锻炼", "Up to date" => "已是最新",
  "Heart Rate" => "心率", "Resting Heart Rate" => "静息心率", "Seven-day steps" => "七日步数",
  "Sleep" => "睡眠", "May be outdated" => "可能已过期", "Steps" => "步数", "Sync Now" => "立即同步",
  "Today" => "今天", "iPhone Health" => "iPhone 健康", "No Synced Health Summary" => "没有已同步的健康摘要",
  "iCloud Sync Unavailable" => "iCloud 同步不可用", "Clear search" => "清除搜索",
  "COMMON DOSAGE" => "常见用量", "LABEL INFORMATION" => "标签信息", "WARNINGS" => "警告",
  "Search Medication Labels" => "搜索药品标签", "No Result" => "无结果",
  "Medication Lookup" => "药品查询", "Search" => "搜索", "Searching FDA labels…" => "正在搜索 FDA 标签…",
  "Actions" => "操作", "Cancel Notification" => "取消通知", "Clear History" => "清除历史",
  "No Matching Notifications" => "没有匹配的通知", "No Notifications" => "没有通知",
  "All" => "全部", "Filter" => "筛选", "Scheduled" => "已计划", "Unread" => "未读",
  "Inbox" => "收件箱", "Loading notifications…" => "正在加载通知…", "Mark All Read" => "全部标为已读",
  "Mark Read" => "标为已读", "No additional details" => "没有更多详情", "Open Conversation" => "打开会话",
  "Enable" => "启用", "Open Settings" => "打开设置", "Remove" => "移除", "Select a Notification" => "选择一条通知",
  "Delivered" => "已送达", "Notifications disabled" => "通知已关闭", "Notifications enabled" => "通知已启用",
  "Notification status unavailable" => "无法获取通知状态", "Notifications not configured" => "通知尚未配置",
  "Agent" => "智能体", "Connections" => "连接", "Data" => "数据", "Diagnostics" => "诊断",
  "General" => "通用", "Providers" => "模型服务", "Default context window" => "默认上下文窗口",
  "Long-term memory" => "长期记忆", "Use long-term memory by default" => "默认使用长期记忆",
  "Use retrieval by default" => "默认使用检索增强", "User preferences" => "用户偏好", "Tokens" => "Token",
  "Default execution" => "默认执行方式", "Debug" => "调试", "Enable debug logging" => "启用调试日志",
  "Info" => "信息", "Log detail" => "日志详细程度", "Open Diagnostics" => "打开诊断",
  "Launch Nola at login" => "登录时启动 Nola", "Daily" => "每天", "Weekly" => "每周",
  "Immediate" => "立即", "Enable notifications" => "启用通知", "Notification frequency" => "通知频率",
  "Open Activity Center" => "打开活动中心", "Open Automations" => "打开自动化中心",
  "API key" => "API 密钥", "Base URL" => "基础 URL", "Model" => "模型", "Open Model Center" => "打开模型中心",
  "Save API Provider" => "保存 API 服务", "Authorization" => "授权", "Connected Host" => "已连接宿主机",
  "Context Window" => "上下文窗口", "Conversation Defaults" => "会话默认设置", "Data Sources" => "数据源",
  "Delivery" => "通知方式", "Diagnostic Tools" => "诊断工具", "Events" => "事件", "Execution" => "执行",
  "Logging" => "日志", "Long-term Memory" => "长期记忆", "API Provider" => "API 服务",
  "Related Tools" => "相关工具", "Startup" => "启动", "Saved" => "已保存", "Settings" => "设置",
  "Automatic" => "自动", "Text" => "文本", "Source refreshed" => "数据源已刷新",
  "Connection verified" => "连接已验证", "Schedule not set" => "未设置计划", "Once" => "一次",
  "Workspace Is Clean" => "工作区没有改动", "Commit All Changes" => "提交全部改动",
  "Creating commit…" => "正在创建提交…", "Commit failed" => "提交失败", "Commit summary" => "提交摘要",
  "Copy Relative Path" => "复制相对路径", "No Text Diff" => "没有文本差异", "Loading diff…" => "正在加载差异…",
  "Files" => "文件", "Reading workspace…" => "正在读取工作区…", "No Repository" => "没有代码仓库",
  "Open" => "打开", "Open Repository in Finder" => "在 Finder 中打开仓库", "Push" => "推送",
  "Push failed" => "推送失败", "Push the current branch" => "推送当前分支", "Quick Look" => "快速查看",
  "Refresh Changes" => "刷新改动", "The remote is up to date." => "远程仓库已是最新。",
  "Reveal in Finder" => "在 Finder 中显示", "Review Changes" => "检查改动", "Added" => "已添加",
  "Conflict" => "冲突", "Deleted" => "已删除", "Modified" => "已修改", "Renamed" => "已重命名",
  "Staged" => "已暂存", "Untracked" => "未跟踪", "Workspace Changes" => "工作区改动"
}.freeze

catalog = JSON.parse(File.read(CATALOG_PATH))
strings = catalog["strings"] ||= {}
memory = existing_translation_memory(catalog)
merge_cross_platform_memory!(memory)
merge_curated_memory!(memory)
source_fallbacks = source_pairs

KEY_DEFAULTS.each do |key, english|
  entry = strings[key] ||= { "localizations" => {} }
  localizations = entry["localizations"] ||= {}
  current = localizations.dig("en", "stringUnit", "value")
  if current.nil? || current.empty? || current == key
    localizations["en"] = { "stringUnit" => { "state" => "translated", "value" => english } }
  end
end
added = 0

source_fallbacks.each do |key, fallback|
  next if strings.key?(key)
  localizations = {}
  LOCALES.each do |locale|
    value = if locale == "en"
      fallback
    elsif locale == "zh-Hans" && ZH_HANS.key?(fallback)
      ZH_HANS.fetch(fallback)
    else
      memory.dig(locale, fallback) || fallback
    end
    translated = locale == "en" || value != fallback
    localizations[locale] = {
      "stringUnit" => {
        "state" => translated ? "translated" : "needs_review",
        "value" => value,
      },
    }
  end
  strings[key] = { "localizations" => localizations }
  added += 1
end

# Earlier versions only applied source fallbacks when adding a new key. Once
# Xcode had already extracted a key, its English value could therefore remain
# the internal identifier forever (for example `mac.data_center.title`). Repair
# existing entries from the same Swift fallback source, then use a reviewed
# catalog comment for model-backed descriptors that do not call a text helper.
strings.each do |key, entry|
  current = entry.dig("localizations", "en", "stringUnit", "value").to_s
  next unless current.empty? || current == key
  fallback = KEY_DEFAULTS[key] || source_fallbacks[key]
  comment = entry["comment"].to_s
  fallback ||= comment unless comment.empty? || comment == key
  next if fallback.nil? || fallback.empty? || fallback == key
  entry["localizations"] ||= {}
  entry["localizations"]["en"] = {
    "stringUnit" => { "state" => "translated", "value" => fallback },
  }
end

# Reuse verified translations for older catalog entries that predate a locale.
strings.each do |key, entry|
  localizations = entry["localizations"] ||= {}
  english = localizations.dig("en", "stringUnit", "value") || key
  LOCALES.each do |locale|
    current = localizations.dig(locale, "stringUnit")
    current_value = current && current["value"].to_s
    next if current && current["state"] == "translated" && !current_value.empty? &&
      (locale == "en" || current_value != english)
    value = if locale == "en"
      english
    elsif locale == "zh-Hans" && ZH_HANS.key?(english)
      ZH_HANS.fetch(english)
    else
      memory.dig(locale, english)
    end
    next if value.nil? || value.empty? || (locale != "en" && value == english)
    localizations[locale] = { "stringUnit" => { "state" => "translated", "value" => value } }
  end
end

File.write(CATALOG_PATH, JSON.pretty_generate(catalog) + "\n")
puts "Added #{added} Mac localization keys; untranslated locale fallbacks remain marked needs_review."
