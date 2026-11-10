import type { Locale } from '../i18n';

const languages: readonly Locale[] = ['zh-Hans', 'en', 'de', 'es', 'fr', 'ja', 'ru'];
type Translations = readonly [string, string, string, string, string, string, string];
const entries: Record<string, Translations> = {
    'local.recommended': ['适合这台设备', 'Recommended for this host', 'Für diesen Host empfohlen', 'Recomendado para este equipo', 'Recommandé pour cet hôte', 'このホストにおすすめ', 'Рекомендуется для этого устройства'],
    'local.memory': ['设备内存', 'Host memory', 'Host-Speicher', 'Memoria del equipo', 'Mémoire de l’hôte', 'ホストのメモリ', 'Память устройства'],
    'local.context': ['上下文', 'Context', 'Kontext', 'Contexto', 'Contexte', 'コンテキスト', 'Контекст'],
    'local.runtime': ['预计运行内存', 'Estimated runtime memory', 'Geschätzter Laufzeitspeicher', 'Memoria estimada en uso', 'Mémoire d’exécution estimée', '推定実行メモリ', 'Оценка памяти при работе'],
    'local.download_size': ['预计下载', 'Estimated download', 'Geschätzter Download', 'Descarga estimada', 'Téléchargement estimé', '推定ダウンロード', 'Оценка загрузки'],
    'local.required_memory': ['建议内存', 'Recommended memory', 'Empfohlener Speicher', 'Memoria recomendada', 'Mémoire recommandée', '推奨メモリ', 'Рекомендуемая память'],
    'local.engine_status': ['运行状态', 'Runtime status', 'Laufzeitstatus', 'Estado de ejecución', 'État de l’exécution', '実行状態', 'Состояние запуска'],
    'local.running_model': ['正在运行：{name}', 'Running: {name}', 'Aktiv: {name}', 'En ejecución: {name}', 'En cours : {name}', '実行中：{name}', 'Запущена: {name}'],
    'local.storage': ['本地模型存储', 'Local model storage', 'Lokaler Modellspeicher', 'Almacenamiento local de modelos', 'Stockage local des modèles', 'ローカルモデルの保存先', 'Хранилище локальных моделей'],
    'local.unavailable': ['不可用', 'Unavailable', 'Nicht verfügbar', 'No disponible', 'Indisponible', '利用不可', 'Недоступно'],
    'local.configuration_error': ['无法读取模型配置：{detail}', 'Could not read model configuration: {detail}', 'Modellkonfiguration konnte nicht gelesen werden: {detail}', 'No se pudo leer la configuración del modelo: {detail}', 'Impossible de lire la configuration du modèle : {detail}', 'モデル設定を読み込めません：{detail}', 'Не удалось прочитать конфигурацию модели: {detail}'],
};

export const localModelCenterMessages = Object.fromEntries(languages.map((language, index) => [language,
    Object.fromEntries(Object.entries(entries).map(([key, values]) => [key, values[index]])),
])) as Record<Locale, Record<string, string>>;
