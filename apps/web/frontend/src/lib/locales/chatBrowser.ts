import type { Locale } from '../i18n';

const languages: Locale[] = ['zh-Hans', 'en', 'de', 'es', 'fr', 'ja', 'ru'];
const entries: Record<string, readonly [string, string, string, string, string, string, string]> = {
    title: ['浏览器', 'Browser', 'Browser', 'Navegador', 'Navigateur', 'ブラウザー', 'Браузер'],
    refresh: ['刷新', 'Refresh', 'Aktualisieren', 'Actualizar', 'Actualiser', '更新', 'Обновить'],
    managed: ['托管浏览器', 'Managed browser', 'Verwalteter Browser', 'Navegador administrado', 'Navigateur géré', '管理ブラウザー', 'Управляемый браузер'],
    active: ['已连接', 'Connected', 'Verbunden', 'Conectado', 'Connecté', '接続済み', 'Подключено'],
    available: ['可用', 'Available', 'Verfügbar', 'Disponible', 'Disponible', '利用可能', 'Доступно'],
    pages: ['会话页面', 'Conversation pages', 'Konversationsseiten', 'Páginas de la conversación', 'Pages de la conversation', '会話のページ', 'Страницы беседы'],
    untitled: ['未命名页面', 'Untitled page', 'Unbenannte Seite', 'Página sin título', 'Page sans titre', '無題のページ', 'Страница без названия'],
    noPages: ['已加载的消息中没有浏览器页面记录。', 'No browser pages were found in the loaded messages.', 'In den geladenen Nachrichten wurden keine Browserseiten gefunden.', 'No hay páginas del navegador en los mensajes cargados.', 'Aucune page de navigateur dans les messages chargés.', '読み込んだメッセージにブラウザーページはありません。', 'В загруженных сообщениях нет страниц браузера.'],
    previewPending: ['已加载的浏览器活动中没有可用预览。', 'No preview is available in the loaded browser activity.', 'In den geladenen Browseraktivitäten ist keine Vorschau verfügbar.', 'No hay vista previa disponible en la actividad cargada.', 'Aucun aperçu n’est disponible dans l’activité chargée.', '読み込まれたブラウザー操作にはプレビューがありません。', 'В загруженных действиях браузера нет предпросмотра.'],
    previewUnavailable: ['此预览只在运行浏览器的本机可用。', 'This preview is available only on the machine running the browser.', 'Diese Vorschau ist nur auf dem Rechner mit dem Browser verfügbar.', 'Esta vista previa solo está disponible en el equipo que ejecuta el navegador.', 'Cet aperçu est disponible uniquement sur l’ordinateur qui exécute le navigateur.', 'このプレビューはブラウザーを実行しているマシンでのみ利用できます。', 'Предпросмотр доступен только на компьютере, где запущен браузер.'],
    previewTitle: ['托管浏览器实时预览', 'Managed browser live preview', 'Live-Vorschau des verwalteten Browsers', 'Vista en vivo del navegador administrado', 'Aperçu en direct du navigateur géré', '管理ブラウザーのライブプレビュー', 'Предпросмотр управляемого браузера'],
    expand: ['展开', 'Expand', 'Vergrößern', 'Ampliar', 'Agrandir', '拡大', 'Развернуть'],
    collapse: ['收起', 'Collapse', 'Verkleinern', 'Reducir', 'Réduire', '閉じる', 'Свернуть'],
    switchFailed: ['无法切换浏览器页面，请刷新后重试。', 'Could not switch browser page. Refresh and try again.', 'Browserseite konnte nicht gewechselt werden. Bitte aktualisieren und erneut versuchen.', 'No se pudo cambiar de página. Actualiza e inténtalo de nuevo.', 'Impossible de changer de page. Actualisez puis réessayez.', 'ブラウザーページを切り替えられませんでした。更新して再試行してください。', 'Не удалось переключить страницу браузера. Обновите и повторите попытку.'],
    native: ['浏览器 Host', 'Browser host', 'Browser-Host', 'Host del navegador', 'Hôte du navigateur', 'ブラウザー Host', 'Хост браузера'],
    connected: ['已连接', 'Connected', 'Verbunden', 'Conectado', 'Connecté', '接続済み', 'Подключено'],
    disconnected: ['未连接', 'Not connected', 'Nicht verbunden', 'Sin conexión', 'Non connecté', '未接続', 'Не подключено'],
    loading: ['正在检查连接…', 'Checking connection…', 'Verbindung wird geprüft…', 'Comprobando conexión…', 'Vérification de la connexion…', '接続を確認中…', 'Проверка подключения…'],
    loadFailed: ['无法获取浏览器连接状态，请重试。', 'Could not load browser connection status. Try again.', 'Browserstatus konnte nicht geladen werden. Erneut versuchen.', 'No se pudo cargar la conexión. Inténtalo de nuevo.', 'Impossible de charger la connexion. Réessayez.', '接続状態を取得できませんでした。再試行してください。', 'Не удалось загрузить состояние подключения. Повторите попытку.'],
};

export const chatBrowserMessages: Record<Locale, Record<string, string>> = Object.fromEntries(languages.map((language, index) => [language,
    Object.fromEntries(Object.entries(entries).map(([key, values]) => [`chatBrowser.${key}`, values[index]])),
])) as Record<Locale, Record<string, string>>;
