import type { Locale } from '../i18n';
const languages: Locale[] = ['zh-Hans', 'en', 'de', 'es', 'fr', 'ja', 'ru'];
const entries: Record<string, string[]> = {
    'state.stopped_locally': ['已停止本机管理', 'Local management stopped', 'Lokale Verwaltung beendet', 'Gestión local detenida', 'Gestion locale arrêtée', 'ローカル管理を停止', 'Локальное управление остановлено'],
    'state.source_callback_completed': ['来源回调已完成', 'Source callback completed', 'Quell-Callback abgeschlossen', 'Devolución del origen completada', 'Retour source terminé', 'ソースのコールバック完了', 'Callback источника завершён'],
"openCodeTitle": ["OpenCode 提供方 OAuth","OpenCode provider OAuth","OpenCode-Provider-OAuth","OAuth de proveedores OpenCode","OAuth fournisseur OpenCode","OpenCode プロバイダー OAuth","OAuth провайдера OpenCode"],
"openCodeLimits": ["仅限本机 Host。插件凭据归 OpenCode。本操作不启用 Nola 推理，也不导入订阅令牌。请使用有密码保护且支持公开 provider OAuth API 的 OpenCode 服务。","Local Host only. OpenCode owns plugin credentials. This does not enable Nola inference or import subscription tokens. Use a password-protected OpenCode server with the published provider OAuth API.","Nur lokaler Host. Plugin-Zugangsdaten gehören OpenCode. Keine Nola-Inferenz oder Übernahme von Abo-Tokens. Passwortgeschützten OpenCode-Server mit öffentlicher Provider-OAuth-API verwenden.","Solo Host local. OpenCode posee las credenciales de plugins. No habilita inferencia Nola ni importa tokens de suscripción. Usa un servidor OpenCode protegido por contraseña con la API OAuth pública.","Host local uniquement. OpenCode gère les identifiants des plugins. Aucune inférence Nola ni importation de jetons d’abonnement. Utilisez un serveur OpenCode protégé par mot de passe avec l’API OAuth publique.","ローカル Host 限定。プラグイン認証情報は OpenCode が管理します。Nola 推論の有効化やサブスクトークンの取込みは行いません。公開 OAuth API 対応のパスワード保護された OpenCode サーバーを使用してください。","Только локальный Host. Данные плагинов принадлежат OpenCode. Инференс Nola не включается, токены подписки не импортируются. Используйте защищённый паролем сервер OpenCode с публичным OAuth API."],
"selectOwner": ["选择 owner","Select an owner","Owner wählen","Elegir propietario","Choisir un propriétaire","owner を選択","Выберите владельца"],
"inspectOwner": ["检查所选 owner","Inspect selected owner","Gewählten Owner prüfen","Inspeccionar propietario","Inspecter le propriétaire","選択した owner を確認","Проверить выбранного владельца"],
"connectOwner": ["连接明确指定的本机 owner","Connect explicit local owner","Expliziten lokalen Owner verbinden","Conectar propietario local explícito","Connecter un propriétaire local explicite","明示したローカル owner に接続","Подключить явно выбранного локального владельца"],
"directory": ["OpenCode 目录（绝对路径）","Absolute OpenCode directory","Absolutes OpenCode-Verzeichnis","Directorio absoluto de OpenCode","Dossier OpenCode absolu","OpenCode ディレクトリ（絶対パス）","Каталог OpenCode (абсолютный путь)"],
"username": ["服务用户名","Server username","Server-Benutzername","Usuario del servidor","Utilisateur du serveur","サーバーユーザー名","Имя пользователя сервера"],
"password": ["服务密码","Server password","Server-Passwort","Contraseña del servidor","Mot de passe du serveur","サーバーパスワード","Пароль сервера"],
"openCodeConsent": ["允许此 Host 访问明确选择的本机 OpenCode 服务与目录。","Allow this Host to access the selected local OpenCode service and directory.","Diesem Host Zugriff auf gewählten lokalen OpenCode-Dienst und Verzeichnis erlauben.","Permitir a este Host acceder al servicio OpenCode local y directorio elegidos.","Autoriser ce Host à accéder au service OpenCode local et au dossier choisis.","この Host が選択したローカル OpenCode サービスとディレクトリにアクセスすることを許可します。","Разрешить этому Host доступ к выбранной локальной службе и каталогу OpenCode."],
"providerMethod": ["Provider / 方法","Provider / method","Provider / Methode","Proveedor / método","Fournisseur / méthode","プロバイダー / 方法","Провайдер / метод"],
"selectMethod": ["选择 OAuth 方法","Select OAuth method","OAuth-Methode wählen","Elegir método OAuth","Choisir la méthode OAuth","OAuth 方法を選択","Выберите метод OAuth"],
"select": ["选择","Select","Auswählen","Seleccionar","Choisir","選択","Выбрать"],
"openCodeLoginConsent": ["在所选 owner 中授权此 provider。其他客户端可替换其待登录流程；OpenCode 不提供按 flow 取消。","Authorize this provider in the selected owner. Another client can replace its pending login; OpenCode has no flow-bound cancellation.","Provider im gewählten Owner autorisieren. Ein anderer Client kann die ausstehende Anmeldung ersetzen; OpenCode bietet keinen Flow-gebundenen Abbruch.","Autorizar este proveedor en el propietario elegido. Otro cliente puede reemplazar el acceso pendiente; OpenCode no permite cancelar por flujo.","Autoriser ce fournisseur dans le propriétaire choisi. Un autre client peut remplacer la connexion en attente ; OpenCode n’offre pas d’annulation liée au flux.","選択した owner でこのプロバイダーを認証します。他のクライアントが保留中のログインを置換できます。OpenCode にフロー単位の取消はありません。","Авторизовать провайдера у выбранного владельца. Другой клиент может заменить ожидающий вход; OpenCode не поддерживает отмену конкретного потока."],
"startSelected": ["开始所选登录","Start selected login","Gewählte Anmeldung starten","Iniciar acceso seleccionado","Démarrer la connexion choisie","選択したログインを開始","Начать выбранный вход"],
"providerStatus": ["Provider：{provider} · 状态：{state}","Provider: {provider} · State: {state}","Provider: {provider} · Status: {state}","Proveedor: {provider} · Estado: {state}","Fournisseur : {provider} · État : {state}","プロバイダー：{provider} · 状態：{state}","Провайдер: {provider} · Состояние: {state}"],
"openAuthorization": ["打开授权：{host}","Open authorization: {host}","Autorisierung öffnen: {host}","Abrir autorización: {host}","Ouvrir l’autorisation : {host}","認証を開く：{host}","Открыть авторизацию: {host}"],
"code": ["授权码","Authorization code","Autorisierungscode","Código de autorización","Code d’autorisation","認証コード","Код авторизации"],
"callbackConsent": ["完成所选 provider 当前待登录流程。请确认未被其他客户端改变；OpenCode 无法将此回调与 flow 原子绑定。","Complete the selected provider’s current pending login. Ensure no other client changed it; OpenCode cannot atomically bind this callback to a flow.","Aktuell ausstehende Anmeldung des gewählten Providers abschließen. Änderungen anderer Clients ausschließen; OpenCode kann den Callback nicht atomar an einen Flow binden.","Completar el acceso pendiente actual del proveedor elegido. Comprueba que ningún cliente lo cambió; OpenCode no puede vincular atómicamente la devolución a un flujo.","Terminer la connexion actuellement en attente du fournisseur choisi. Vérifiez qu’aucun client ne l’a modifiée ; OpenCode ne peut pas lier atomiquement ce retour à un flux.","選択したプロバイダーの現在の保留ログインを完了します。他のクライアントが変更していないことを確認してください。OpenCode はコールバックをフローに原子的に紐付けられません。","Завершить текущий ожидающий вход провайдера. Убедитесь, что другой клиент его не изменил: OpenCode не может атомарно связать callback с потоком."],
"confirmCallback": ["确认回调","Confirm callback","Callback bestätigen","Confirmar devolución","Confirmer le retour","コールバックを確認","Подтвердить callback"],
"forgetHandle": ["忘记本机句柄（不取消来源登录）","Forget local handle — does not cancel source login","Lokalen Handle vergessen — Quellanmeldung bleibt aktiv","Olvidar identificador local — no cancela el acceso de origen","Oublier l’identifiant local — sans annuler la connexion source","ローカルハンドルを破棄（ソースログインは取消しません）","Забыть локальный дескриптор — вход источника не отменяется"],
"openCodeError": ["owner 或登录待确认，未自动重试。开始新登录前请检查所选 OpenCode owner。","Owner or login unconfirmed. No automatic retry was sent. Inspect the selected OpenCode owner before another login.","Owner oder Anmeldung unbestätigt. Kein automatischer Neuversuch. Gewählten OpenCode-Owner vor neuer Anmeldung prüfen.","Propietario o acceso sin confirmar. Sin reintento automático. Inspecciona el propietario OpenCode antes de otro acceso.","Propriétaire ou connexion non confirmé. Aucun nouvel essai automatique. Inspectez le propriétaire OpenCode avant une autre connexion.","owner またはログインを確認できません。自動再試行はしていません。新しいログイン前に選択した OpenCode owner を確認してください。","Владелец или вход не подтверждены. Автоповтор не отправлен. Перед новым входом проверьте выбранного владельца OpenCode."],
"state.authenticated_source_only": ["来源已登录（仅来源）","Signed in to source only","Nur an Quelle angemeldet","Acceso solo al origen","Connecté à la source uniquement","ソースのみログイン済み","Вход выполнен только в источнике"],
"action.login_in_codex_then_refresh": ["请在 Codex 登录，再重新读取状态。","Sign in to Codex, then read status again.","Bei Codex anmelden, dann Status erneut lesen.","Inicia sesión en Codex y vuelve a leer el estado.","Connectez-vous à Codex, puis relisez l’état.","Codex にログインしてから状態を再取得してください。","Войдите в Codex и снова прочитайте состояние."],
"action.login_in_kimi_code_then_refresh": ["请在 Kimi Code 登录，再重新读取状态。","Sign in to Kimi Code, then read status again.","Bei Kimi Code anmelden, dann Status erneut lesen.","Inicia sesión en Kimi Code y vuelve a leer el estado.","Connectez-vous à Kimi Code, puis relisez l’état.","Kimi Code にログインしてから状態を再取得してください。","Войдите в Kimi Code и снова прочитайте состояние."],
"action.use_explicit_api_key_connection_for_inference": ["推理请使用明确配置的 API key 连接；来源登录不提供推理授权。","Use an explicit API key connection for inference; source sign-in does not authorize inference.","Für Inferenz eine explizite API-Key-Verbindung verwenden; Quellanmeldung autorisiert keine Inferenz.","Usa una conexión explícita con clave API para inferencia; el acceso al origen no la autoriza.","Utilisez une connexion explicite par clé API pour l’inférence ; la connexion source ne l’autorise pas.","推論には明示的な API キー接続を使ってください。ソースへのログインは推論を許可しません。","Для инференса используйте явно настроенное подключение с API-ключом; вход в источник не даёт такого разрешения."],
"action.check_explicit_source_credential_file": ["请核对明确选择的来源凭据文件。","Check the explicitly selected source credential file.","Explizit gewählte Quelldatei für Zugangsdaten prüfen.","Comprueba el archivo de credenciales de origen elegido.","Vérifiez le fichier d’identifiants source explicitement choisi.","明示的に選択したソース認証ファイルを確認してください。","Проверьте явно выбранный файл учётных данных источника."],
    "codexTitle": [
        "Codex 本机登录",
        "Codex local sign-in",
        "Lokale Codex-Anmeldung",
        "Inicio local de Codex",
        "Connexion locale Codex",
        "Codex のローカルログイン",
        "Локальный вход Codex"
    ],
    "codexLimits": [
        "仅限本机 Host。选择 Codex 程序和现有主目录。登录与令牌刷新归 Codex；Nola 不接收令牌，也不启用订阅 HTTP 推理。",
        "Local Host only. Select Codex and its existing home. Codex owns login and token refresh; Nola receives no tokens and does not enable subscription HTTP inference.",
        "Nur lokaler Host. Codex und vorhandenes Home wählen. Codex verwaltet Anmeldung und Token-Erneuerung; Nola erhält keine Tokens und aktiviert keine HTTP-Inferenz per Abo.",
        "Solo Host local. Elige Codex y su directorio existente. Codex gestiona el acceso y renueva tokens; Nola no los recibe ni habilita inferencia HTTP de suscripción.",
        "Host local uniquement. Choisissez Codex et son dossier existant. Codex gère la connexion et les jetons ; Nola ne reçoit aucun jeton et n’active pas l’inférence HTTP par abonnement.",
        "ローカル Host 限定。Codex と既存ホームを選択します。ログインとトークン更新は Codex が管理します。Nola はトークンを受け取らず、サブスク HTTP 推論も有効にしません。",
        "Только локальный Host. Выберите Codex и существующий каталог. Входом и обновлением токенов управляет Codex; Nola не получает токены и не включает HTTP-инференс по подписке."
    ],
    "account": [
        "Nola 来源账户",
        "Nola source account",
        "Nola-Quellkonto",
        "Cuenta de origen de Nola",
        "Compte source Nola",
        "Nola ソースアカウント",
        "Учётная запись источника Nola"
    ],
    "newAccount": [
        "创建来源账户",
        "Create a source account",
        "Quellkonto erstellen",
        "Crear cuenta de origen",
        "Créer un compte source",
        "ソースアカウントを作成",
        "Создать учётную запись источника"
    ],
    "label": [
        "账户名称",
        "Account label",
        "Kontobezeichnung",
        "Nombre de cuenta",
        "Nom du compte",
        "アカウント名",
        "Название учётной записи"
    ],
    "executable": [
        "Codex 程序（绝对路径）",
        "Codex executable (absolute path)",
        "Codex-Programm (absoluter Pfad)",
        "Ejecutable Codex (ruta absoluta)",
        "Exécutable Codex (chemin absolu)",
        "Codex 実行ファイル（絶対パス）",
        "Программа Codex (абсолютный путь)"
    ],
    "home": [
        "CODEX_HOME（绝对路径）",
        "CODEX_HOME (absolute path)",
        "CODEX_HOME (absoluter Pfad)",
        "CODEX_HOME (ruta absoluta)",
        "CODEX_HOME (chemin absolu)",
        "CODEX_HOME（絶対パス）",
        "CODEX_HOME (абсолютный путь)"
    ],
    "codexConsent": [
        "允许所选本机 Codex 管理登录。新登录可能替换该主目录的当前账户。",
        "Allow the selected local Codex to manage login. A new sign-in may replace that home’s active account.",
        "Dem gewählten lokalen Codex die Anmeldung erlauben. Eine neue Anmeldung kann das aktive Konto dieses Home ersetzen.",
        "Permitir que el Codex local elegido gestione el acceso. Un nuevo inicio puede reemplazar la cuenta activa de ese directorio.",
        "Autoriser le Codex local choisi à gérer la connexion. Une nouvelle connexion peut remplacer le compte actif de ce dossier.",
        "選択したローカル Codex にログイン管理を許可します。新しいログインでそのホームの現在のアカウントが置き換わる場合があります。",
        "Разрешить выбранному локальному Codex управлять входом. Новый вход может заменить активную запись этого каталога."
    ],
    "startCodex": [
        "开始 Codex 登录",
        "Start Codex sign-in",
        "Codex-Anmeldung starten",
        "Iniciar acceso a Codex",
        "Démarrer la connexion Codex",
        "Codex ログインを開始",
        "Начать вход Codex"
    ],
    "refreshCodex": [
        "刷新 Codex 管理的令牌",
        "Refresh Codex-owned token",
        "Codex-Token erneuern",
        "Renovar token de Codex",
        "Renouveler le jeton géré par Codex",
        "Codex 管理トークンを更新",
        "Обновить токен Codex"
    ],
    "cancelCodex": [
        "取消本机登录",
        "Cancel local sign-in",
        "Lokale Anmeldung abbrechen",
        "Cancelar inicio local",
        "Annuler la connexion locale",
        "ローカルログインを取消",
        "Отменить локальный вход"
    ],
    "checkCodex": [
        "核对此次登录",
        "Check this sign-in",
        "Diese Anmeldung prüfen",
        "Comprobar este inicio",
        "Vérifier cette connexion",
        "このログインを確認",
        "Проверить этот вход"
    ],
    "disabled": [
        "订阅推理未启用",
        "Subscription inference disabled",
        "Abo-Inferenz deaktiviert",
        "Inferencia de suscripción desactivada",
        "Inférence par abonnement désactivée",
        "サブスク推論は無効",
        "Инференс по подписке отключён"
    ],
    "continueCodex": [
        "继续 OpenAI 登录",
        "Continue in OpenAI sign-in",
        "Bei OpenAI weiter anmelden",
        "Continuar en OpenAI",
        "Continuer chez OpenAI",
        "OpenAI ログインに進む",
        "Продолжить вход OpenAI"
    ],
    "codexError": [
        "无法确认登录。重试前请检查已保存账户。",
        "Could not confirm login. Check the saved account before retrying.",
        "Anmeldung nicht bestätigt. Vor erneutem Versuch gespeichertes Konto prüfen.",
        "No se confirmó el acceso. Comprueba la cuenta guardada antes de reintentar.",
        "Connexion non confirmée. Vérifiez le compte enregistré avant de réessayer.",
        "ログインを確認できません。再試行前に保存済みアカウントを確認してください。",
        "Вход не подтверждён. Перед повтором проверьте сохранённую запись."
    ],
    "codexCancelLimits": [
        "取消只停止本次 Nola 操作，不撤销 Codex 已完成的登录。此面板不管理 OpenCode、Kimi 或 Nola 独立 ChatGPT 授权。",
        "Cancellation stops this Nola operation, not a login already completed by Codex. This panel does not manage OpenCode, Kimi or independent Nola ChatGPT authorization.",
        "Abbrechen beendet nur diesen Nola-Vorgang, nicht eine abgeschlossene Codex-Anmeldung. Dieses Panel verwaltet weder OpenCode, Kimi noch eigene ChatGPT-Autorisierung für Nola.",
        "Cancelar detiene esta operación de Nola, no un acceso ya completado por Codex. Este panel no gestiona OpenCode, Kimi ni autorización ChatGPT independiente de Nola.",
        "L’annulation arrête cette opération Nola, sans annuler une connexion Codex terminée. Ce panneau ne gère pas OpenCode, Kimi ni l’autorisation ChatGPT indépendante de Nola.",
        "取消は今回の Nola 操作だけを停止し、Codex の完了済みログインは元に戻しません。このパネルは OpenCode、Kimi、Nola 独自の ChatGPT 認証を管理しません。",
        "Отмена останавливает только эту операцию Nola, но не отменяет завершённый вход Codex. Панель не управляет OpenCode, Kimi или отдельной авторизацией ChatGPT для Nola."
    ],
    "kimiTitle": [
        "Kimi Code 本机服务登录",
        "Kimi Code local service sign-in",
        "Anmeldung am lokalen Kimi-Code-Dienst",
        "Acceso al servicio local Kimi Code",
        "Connexion au service local Kimi Code",
        "Kimi Code ローカルサービスへのログイン",
        "Вход в локальную службу Kimi Code"
    ],
    "kimiLimits": [
        "连接此 Host 上已运行且需认证的 Kimi 服务。使用本机服务 bearer，而非 OAuth 令牌。Nola 不扫描文件、不刷新 OAuth 令牌，也不运行 Kimi Agent 会话。",
        "Connect an already-running authenticated Kimi service on this Host. Use its local server bearer, not an OAuth token. Nola does not scan files, refresh OAuth tokens or run Kimi Agent sessions.",
        "Mit einem laufenden authentifizierten Kimi-Dienst dieses Hosts verbinden. Lokales Server-Bearer statt OAuth-Token verwenden. Nola durchsucht keine Dateien, erneuert keine OAuth-Tokens und startet keine Kimi-Agent-Sitzungen.",
        "Conecta un servicio Kimi autenticado ya activo en este Host. Usa el bearer local, no un token OAuth. Nola no busca archivos, renueva tokens OAuth ni ejecuta sesiones de Kimi Agent.",
        "Connectez un service Kimi authentifié déjà actif sur ce Host. Utilisez son bearer local, pas un jeton OAuth. Nola ne parcourt pas les fichiers, ne renouvelle pas OAuth et n’exécute pas de sessions Kimi Agent.",
        "この Host で稼働中の認証付き Kimi サービスに接続します。OAuth トークンではなくローカルサーバー bearer を使います。Nola はファイル検索、OAuth 更新、Kimi Agent セッション実行を行いません。",
        "Подключите уже работающую службу Kimi с аутентификацией на этом Host. Используйте локальный server bearer, не OAuth-токен. Nola не сканирует файлы, не обновляет OAuth и не запускает сеансы Kimi Agent."
    ],
    "savedOwner": [
        "已保存的本机 owner",
        "Saved local owner",
        "Gespeicherter lokaler Owner",
        "Propietario local guardado",
        "Propriétaire local enregistré",
        "保存済みローカル owner",
        "Сохранённый локальный владелец"
    ],
    "newKimi": [
        "新建 Kimi owner",
        "New Kimi owner",
        "Neuer Kimi-Owner",
        "Nuevo propietario Kimi",
        "Nouveau propriétaire Kimi",
        "新しい Kimi owner",
        "Новый владелец Kimi"
    ],
    "refreshOwners": [
        "刷新已保存 owner",
        "Refresh saved owners",
        "Gespeicherte Owner aktualisieren",
        "Actualizar propietarios",
        "Actualiser les propriétaires",
        "保存済み owner を更新",
        "Обновить владельцев"
    ],
    "origin": [
        "Loopback 服务地址",
        "Loopback service origin",
        "Loopback-Dienstadresse",
        "Origen del servicio loopback",
        "Origine du service loopback",
        "ループバックサービスのオリジン",
        "Адрес службы loopback"
    ],
    "bearer": [
        "本机服务 bearer",
        "Local service bearer",
        "Lokales Dienst-Bearer",
        "Bearer del servicio local",
        "Bearer du service local",
        "ローカルサービス bearer",
        "Bearer локальной службы"
    ],
    "inspectConsent": [
        "允许对所选本机服务进行认证状态与协议检查。",
        "Allow authenticated status and protocol inspection of this selected local service.",
        "Authentifizierte Status- und Protokollprüfung dieses lokalen Dienstes erlauben.",
        "Permitir inspección autenticada del estado y protocolo de este servicio local.",
        "Autoriser l’inspection authentifiée de l’état et du protocole de ce service local.",
        "選択したローカルサービスの認証付き状態・プロトコル確認を許可します。",
        "Разрешить проверку состояния и протокола выбранной локальной службы с аутентификацией."
    ],
    "connectKimi": [
        "连接本机 Kimi owner",
        "Connect local Kimi owner",
        "Lokalen Kimi-Owner verbinden",
        "Conectar propietario Kimi local",
        "Connecter le propriétaire Kimi local",
        "ローカル Kimi owner に接続",
        "Подключить локального владельца Kimi"
    ],
    "ownerStatus": [
        "管理地址 {origin} · 服务 {server}",
        "Managed origin {origin} · Server {server}",
        "Verwaltete Adresse {origin} · Server {server}",
        "Origen gestionado {origin} · Servidor {server}",
        "Origine gérée {origin} · Serveur {server}",
        "管理オリジン {origin} · サーバー {server}",
        "Управляемый адрес {origin} · Сервер {server}"
    ],
    "region": [
        "登录区域",
        "Login region",
        "Anmelderegion",
        "Región de acceso",
        "Région de connexion",
        "ログイン地域",
        "Регион входа"
    ],
    "serviceRegion": [
        "使用 Kimi 服务设置",
        "Use Kimi service setting",
        "Kimi-Diensteinstellung verwenden",
        "Usar ajuste del servicio Kimi",
        "Utiliser le réglage Kimi",
        "Kimi サービス設定を使用",
        "Использовать настройку Kimi"
    ],
    "china": [
        "中国大陆",
        "Mainland China",
        "Festlandchina",
        "China continental",
        "Chine continentale",
        "中国本土",
        "Материковый Китай"
    ],
    "global": [
        "全球",
        "Global",
        "Global",
        "Global",
        "Monde",
        "グローバル",
        "Глобальный"
    ],
    "loginConsent": [
        "允许此 Kimi owner 开始其托管 provider 登录。",
        "Allow this Kimi owner to start its managed provider login.",
        "Diesem Kimi-Owner den Start der verwalteten Provider-Anmeldung erlauben.",
        "Permitir a este propietario Kimi iniciar el acceso a su proveedor gestionado.",
        "Autoriser ce propriétaire Kimi à lancer la connexion de son fournisseur géré.",
        "この Kimi owner に管理対象プロバイダーのログイン開始を許可します。",
        "Разрешить этому владельцу Kimi начать вход управляемого провайдера."
    ],
    "startKimi": [
        "开始 Kimi 登录",
        "Start Kimi sign-in",
        "Kimi-Anmeldung starten",
        "Iniciar acceso a Kimi",
        "Démarrer la connexion Kimi",
        "Kimi ログインを開始",
        "Начать вход Kimi"
    ],
    "readState": [
        "读取登录状态",
        "Read login state",
        "Anmeldestatus lesen",
        "Leer estado de acceso",
        "Lire l’état de connexion",
        "ログイン状態を取得",
        "Прочитать состояние входа"
    ],
    "continueKimi": [
        "继续 Kimi 登录",
        "Continue in Kimi sign-in",
        "Bei Kimi weiter anmelden",
        "Continuar en Kimi",
        "Continuer chez Kimi",
        "Kimi ログインに進む",
        "Продолжить вход Kimi"
    ],
    "userCode": [
        "用户代码：{code}",
        "User code: {code}",
        "Benutzercode: {code}",
        "Código de usuario: {code}",
        "Code utilisateur : {code}",
        "ユーザーコード：{code}",
        "Код пользователя: {code}"
    ],
    "stopObservation": [
        "停止 Nola 观察",
        "Stop Nola observation",
        "Nola-Beobachtung beenden",
        "Detener observación de Nola",
        "Arrêter l’observation Nola",
        "Nola の監視を停止",
        "Остановить наблюдение Nola"
    ],
    "cancelConsent": [
        "取消此 Kimi provider 当前的登录。Kimi 不支持原子按 flow 取消；请求到达前，其他客户端可能已替换登录流程。",
        "Cancel this Kimi provider’s current login. Kimi has no atomic flow-specific cancellation; another client may replace the flow before cancellation arrives.",
        "Aktuelle Anmeldung dieses Kimi-Providers abbrechen. Kimi bietet keinen atomaren Flow-Abbruch; ein anderer Client kann den Flow vorher ersetzen.",
        "Cancelar el acceso actual de este proveedor Kimi. No hay cancelación atómica por flujo; otro cliente puede reemplazarlo antes de que llegue la cancelación.",
        "Annuler la connexion actuelle de ce fournisseur Kimi. Pas d’annulation atomique par flux : un autre client peut le remplacer avant l’arrivée de la demande.",
        "この Kimi プロバイダーの現在のログインを取消します。フロー単位の原子的な取消はなく、要求到着前に他のクライアントがフローを置き換える場合があります。",
        "Отменить текущий вход этого провайдера Kimi. Атомарной отмены конкретного потока нет: другой клиент может заменить поток до получения отмены."
    ],
    "cancelProvider": [
        "取消来源 provider 登录",
        "Cancel source provider login",
        "Quellprovider-Anmeldung abbrechen",
        "Cancelar acceso del proveedor de origen",
        "Annuler la connexion du fournisseur source",
        "ソースプロバイダーのログインを取消",
        "Отменить вход провайдера источника"
    ],
    "cancelResult": [
        "来源取消已确认：{answer}。不会撤销已完成的登录。",
        "Source cancellation confirmed: {answer}. Completed login is not undone.",
        "Quellabbruch bestätigt: {answer}. Abgeschlossene Anmeldung bleibt bestehen.",
        "Cancelación del origen confirmada: {answer}. No revierte un acceso completado.",
        "Annulation source confirmée : {answer}. Une connexion terminée reste inchangée.",
        "ソースの取消確認：{answer}。完了済みログインは元に戻りません。",
        "Отмена источника подтверждена: {answer}. Завершённый вход не отменяется."
    ],
    "yes": [
        "是",
        "Yes",
        "Ja",
        "Sí",
        "Oui",
        "はい",
        "Да"
    ],
    "no": [
        "否",
        "No",
        "Nein",
        "No",
        "Non",
        "いいえ",
        "Нет"
    ],
    "kimiError": [
        "无法确认所选服务或流程。未自动重试或发送取消。开始新登录前请先检查 Kimi。",
        "Could not confirm the selected service or flow. No automatic retry or cancellation was sent. Check Kimi before starting another login.",
        "Dienst oder Flow nicht bestätigt. Kein automatischer Wiederholungs- oder Abbruchversuch. Kimi vor neuer Anmeldung prüfen.",
        "No se confirmó el servicio o flujo. No se reintentó ni canceló automáticamente. Revisa Kimi antes de iniciar otro acceso.",
        "Service ou flux non confirmé. Aucun nouvel essai ni annulation automatique. Vérifiez Kimi avant une autre connexion.",
        "選択したサービスまたはフローを確認できません。自動再試行・取消はしていません。次のログイン前に Kimi を確認してください。",
        "Служба или поток не подтверждены. Автоповтор и отмена не отправлялись. Проверьте Kimi перед новым входом."
    ],
    "readAction": [
        "请在来源应用中检查登录；重新读取状态不会刷新令牌或启用推理。",
        "Check sign-in in the source app. Reading status does not refresh tokens or enable inference.",
        "Anmeldung in der Quell-App prüfen. Statuslesen erneuert keine Tokens und aktiviert keine Inferenz.",
        "Comprueba el acceso en la app de origen. Leer el estado no renueva tokens ni habilita inferencia.",
        "Vérifiez la connexion dans l’app source. Lire l’état ne renouvelle pas les jetons et n’active pas l’inférence.",
        "ソースアプリでログインを確認してください。状態取得はトークン更新や推論有効化を行いません。",
        "Проверьте вход в исходном приложении. Чтение состояния не обновляет токены и не включает инференс."
    ],
    "state.starting": [
        "正在开始",
        "Starting",
        "Wird gestartet",
        "Iniciando",
        "Démarrage",
        "開始中",
        "Запуск"
    ],
    "state.pending": [
        "等待登录",
        "Awaiting sign-in",
        "Anmeldung ausstehend",
        "Esperando acceso",
        "Connexion en attente",
        "ログイン待ち",
        "Ожидание входа"
    ],
    "state.authenticated": [
        "来源已认证",
        "Source authenticated",
        "Quelle authentifiziert",
        "Origen autenticado",
        "Source authentifiée",
        "ソース認証済み",
        "Источник аутентифицирован"
    ],
    "state.cancelled": [
        "已取消",
        "Cancelled",
        "Abgebrochen",
        "Cancelado",
        "Annulé",
        "取消済み",
        "Отменено"
    ],
    "state.failed": [
        "失败",
        "Failed",
        "Fehlgeschlagen",
        "Error",
        "Échec",
        "失敗",
        "Ошибка"
    ],
    "state.expired": [
        "已过期",
        "Expired",
        "Abgelaufen",
        "Caducado",
        "Expiré",
        "期限切れ",
        "Срок истёк"
    ],
    "state.revoked": [
        "已撤销",
        "Revoked",
        "Widerrufen",
        "Revocado",
        "Révoqué",
        "失効済み",
        "Отозвано"
    ],
    "state.unauthenticated": [
        "尚未认证",
        "Not authenticated",
        "Nicht authentifiziert",
        "Sin autenticar",
        "Non authentifié",
        "未認証",
        "Не аутентифицировано"
    ],
    "state.denied": [
        "已拒绝",
        "Denied",
        "Abgelehnt",
        "Denegado",
        "Refusé",
        "拒否済み",
        "Отклонено"
    ],
    "state.unconfirmed": [
        "结果待确认",
        "Unconfirmed",
        "Unbestätigt",
        "Sin confirmar",
        "Non confirmé",
        "未確認",
        "Не подтверждено"
    ],
    "state.unknown": [
        "状态未知",
        "Unknown state",
        "Status unbekannt",
        "Estado desconocido",
        "État inconnu",
        "状態不明",
        "Неизвестное состояние"
    ],
    "state.login_required": [
        "需要登录",
        "Sign-in required",
        "Anmeldung erforderlich",
        "Acceso necesario",
        "Connexion requise",
        "ログインが必要",
        "Требуется вход"
    ],
    "state.present_but_unsupported": [
        "存在登录，推理不支持",
        "Login present; inference unsupported",
        "Anmeldung vorhanden; Inferenz nicht unterstützt",
        "Acceso presente; inferencia no compatible",
        "Connexion présente ; inférence non prise en charge",
        "ログインあり・推論非対応",
        "Вход есть; инференс не поддерживается"
    ],
    "state.invalid": [
        "状态无效",
        "Invalid state",
        "Ungültiger Status",
        "Estado inválido",
        "État invalide",
        "無効な状態",
        "Недопустимое состояние"
    ],
    "state.unavailable": [
        "不可用",
        "Unavailable",
        "Nicht verfügbar",
        "No disponible",
        "Indisponible",
        "利用不可",
        "Недоступно"
    ],
    "state.disconnected": [
        "已断开",
        "Disconnected",
        "Getrennt",
        "Desconectado",
        "Déconnecté",
        "切断済み",
        "Отключено"
    ]
};
export const sourceAccountMessages = Object.fromEntries(languages.map((language, index) => [language,
    Object.fromEntries(Object.entries(entries).map(([key, values]) => [`sourceAccounts.${key}`, values[index]])),
])) as Record<Locale, Record<string, string>>;
export function sourceAccountStateKey(state: string): string {
    const key = 'state.' + state;
    return Object.hasOwn(entries, key) ? 'sourceAccounts.' + key : 'sourceAccounts.state.unknown';
}
export function sourceAccountActionKey(action: string | null): string {
    const key = 'action.' + action;
    return Object.hasOwn(entries, key) ? 'sourceAccounts.' + key : 'sourceAccounts.readAction';
}

