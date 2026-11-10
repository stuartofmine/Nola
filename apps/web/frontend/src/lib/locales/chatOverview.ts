import type { Locale } from '../i18n';

const languages: Locale[] = ['zh-Hans', 'en', 'de', 'es', 'fr', 'ja', 'ru'];
const entries: Record<string, readonly [string, string, string, string, string, string, string]> = {
    cacheDiagnostic: ['请求缓存诊断', 'Request cache diagnostics', 'Anfrage-Cache-Diagnose', 'Diagnóstico de caché de la solicitud', 'Diagnostic du cache de requête', 'リクエストキャッシュ診断', 'Диагностика кэша запроса'],
    request: ['请求', 'Request', 'Anfrage', 'Solicitud', 'Requête', 'リクエスト', 'Запрос'],
    attempt: ['物理尝试', 'Physical attempt', 'Physischer Versuch', 'Intento físico', 'Tentative physique', '物理試行', 'Физическая попытка'],
    cacheRate: ['缓存命中率（会话累计）', 'Cache hit rate (conversation total)', 'Cache-Trefferrate (Gespräch gesamt)', 'Aciertos de caché (total de conversación)', 'Taux de cache (total de la conversation)', 'キャッシュ命中率（会話全体）', 'Попадания в кэш (весь диалог)'],
    cacheUnknown: ['未知（未上报）', 'Unknown (not reported)', 'Unbekannt (nicht gemeldet)', 'Desconocido (sin datos)', 'Inconnu (non signalé)', '不明（未報告）', 'Неизвестно (нет данных)'],
    cacheReported: ['已上报请求', 'Reported requests', 'Gemeldete Anfragen', 'Solicitudes con datos', 'Requêtes avec données', '報告済みリクエスト', 'Запросы с данными'],
    cacheCoverage: ['上报覆盖率', 'Reporting coverage', 'Meldeabdeckung', 'Cobertura de datos', 'Couverture des données', '報告率', 'Доля запросов с данными'],
    cacheUnreported: ['未上报请求', 'Unreported requests', 'Nicht gemeldete Anfragen', 'Solicitudes sin datos', 'Requêtes sans données', '未報告リクエスト', 'Запросы без данных'],
    cacheUnavailable: ['统计读取失败', 'Usage unavailable', 'Statistik nicht verfügbar', 'Estadísticas no disponibles', 'Statistiques indisponibles', '統計を読み込めません', 'Статистика недоступна'],
    title: ['概览', 'Overview', 'Übersicht', 'Resumen', 'Vue d’ensemble', '概要', 'Обзор'],
    running: ['运行中', 'Running', 'Läuft', 'En ejecución', 'En cours', '実行中', 'Выполняется'],
    ready: ['就绪', 'Ready', 'Bereit', 'Listo', 'Prêt', '準備完了', 'Готово'],
    localHost: ['本地 Host', 'Local Host', 'Lokaler Host', 'Host local', 'Hôte local', 'ローカル Host', 'Локальный Host'],
    automatic: ['自动选择模型', 'Automatic model', 'Automatisches Modell', 'Modelo automático', 'Modèle automatique', 'モデルを自動選択', 'Автоматическая модель'],
    context: ['上下文', 'Context', 'Kontext', 'Contexto', 'Contexte', 'コンテキスト', 'Контекст'],
    estimated: ['Core 标记为估计值', 'Marked estimated by Core', 'Von Core als Schätzung markiert', 'Core lo marca como estimado', 'Marqué comme estimé par Core', 'Core による推定値', 'Core отмечает как оценку'],
    lastRun: ['上次运行记录', 'Last recorded run', 'Letzter aufgezeichneter Lauf', 'Última ejecución registrada', 'Dernière exécution enregistrée', '前回の実行記録', 'Последний записанный запуск'],
    notRecorded: ['尚无可核实的运行占用', 'No verified run usage yet', 'Noch keine verifizierte Laufnutzung', 'Aún no hay uso verificado de la ejecución', 'Aucune utilisation d’exécution vérifiée pour le moment', '検証済みの実行使用量はまだありません', 'Пока нет подтверждённых данных об использовании'],
    externalUnit: ['外部执行单元的上下文由该单元管理', 'Context is managed by the external execution unit', 'Kontext wird von der externen Ausführungseinheit verwaltet', 'El contexto lo gestiona la unidad de ejecución externa', 'Le contexte est géré par l’unité d’exécution externe', 'コンテキストは外部実行ユニットが管理します', 'Контекстом управляет внешняя исполнительная среда'],
    loading: ['正在读取上下文…', 'Loading context…', 'Kontext wird geladen…', 'Cargando contexto…', 'Chargement du contexte…', 'コンテキストを読み込み中…', 'Загрузка контекста…'],
    loadFailed: ['无法读取上下文。', 'Could not load context.', 'Kontext konnte nicht geladen werden.', 'No se pudo cargar el contexto.', 'Impossible de charger le contexte.', 'コンテキストを読み込めません。', 'Не удалось загрузить контекст.'],
    retry: ['重试', 'Retry', 'Erneut versuchen', 'Reintentar', 'Réessayer', '再試行', 'Повторить'],
};

export const chatOverviewMessages: Record<Locale, Record<string, string>> = Object.fromEntries(languages.map((language, index) => [language,
    Object.fromEntries(Object.entries(entries).map(([key, values]) => [`chatOverview.${key}`, values[index]])),
])) as Record<Locale, Record<string, string>>;
