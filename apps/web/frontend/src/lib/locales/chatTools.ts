import type { Locale } from '../i18n';

const languages: Locale[] = ['zh-Hans', 'en', 'de', 'es', 'fr', 'ja', 'ru'];
const entries: Record<string, readonly [string, string, string, string, string, string, string]> = {
    title: ['工具与连接', 'Tools and connections', 'Werkzeuge und Verbindungen', 'Herramientas y conexiones', 'Outils et connexions', 'ツールと接続', 'Инструменты и подключения'],
    refresh: ['刷新', 'Refresh', 'Aktualisieren', 'Actualizar', 'Actualiser', '更新', 'Обновить'],
    loading: ['正在检查工具状态…', 'Checking tool status…', 'Werkzeugstatus wird geprüft…', 'Comprobando herramientas…', 'Vérification des outils…', 'ツールの状態を確認中…', 'Проверка инструментов…'],
    loadFailed: ['部分工具状态无法获取，请重试。', 'Some tool status could not be loaded. Try again.', 'Ein Teil des Werkzeugstatus konnte nicht geladen werden. Erneut versuchen.', 'No se pudo cargar parte del estado. Inténtalo de nuevo.', 'Certains états n’ont pas pu être chargés. Réessayez.', '一部の状態を取得できませんでした。再試行してください。', 'Не удалось загрузить часть состояний. Повторите попытку.'],
    catalog: ['工具目录', 'Tool catalog', 'Werkzeugkatalog', 'Catálogo de herramientas', 'Catalogue des outils', 'ツールカタログ', 'Каталог инструментов'],
    catalogCount: ['{count} 项', '{count} entries', '{count} Einträge', '{count} elementos', '{count} entrées', '{count} 件', '{count} записей'],
    browser: ['浏览器连接', 'Browser connection', 'Browserverbindung', 'Conexión del navegador', 'Connexion du navigateur', 'ブラウザー接続', 'Подключение браузера'],
    connected: ['已连接', 'Connected', 'Verbunden', 'Conectado', 'Connecté', '接続済み', 'Подключено'],
    disconnected: ['未连接', 'Not connected', 'Nicht verbunden', 'Sin conexión', 'Non connecté', '未接続', 'Не подключено'],
    catalogHint: ['目录仅列出工具定义；实际调用仍由会话权限和执行环境决定。', 'The catalog lists tool definitions, not availability. Session permissions and the execution environment govern each call.', 'Der Katalog listet Werkzeugdefinitionen, nicht deren Verfügbarkeit. Sitzungsrechte und Umgebung bestimmen die Nutzung.', 'El catálogo muestra definiciones, no disponibilidad. Los permisos y el entorno determinan cada uso.', 'Le catalogue liste des définitions, pas leur disponibilité. Les permissions et l’environnement régissent leur utilisation.', 'カタログはツール定義の一覧で、使用可否ではありません。実際の使用は権限と実行環境に従います。', 'Каталог показывает определения, а не доступность. Использование зависит от разрешений и среды.'],
    manage: ['管理工具', 'Manage tools', 'Werkzeuge verwalten', 'Gestionar herramientas', 'Gérer les outils', 'ツールの管理', 'Управление инструментами'],
    systemAccess: ['系统访问', 'System access', 'Systemzugriff', 'Acceso al sistema', 'Accès au système', 'システムアクセス', 'Доступ к системе'],
    skillsMcp: ['Skills 与 MCP', 'Skills and MCP', 'Skills und MCP', 'Skills y MCP', 'Skills et MCP', 'Skills と MCP', 'Навыки и MCP'],
};

export const chatToolsMessages: Record<Locale, Record<string, string>> = Object.fromEntries(languages.map((language, index) => [language,
    Object.fromEntries(Object.entries(entries).map(([key, values]) => [`chatTools.${key}`, values[index]])),
])) as Record<Locale, Record<string, string>>;
