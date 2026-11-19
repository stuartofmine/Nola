export type DesktopStrings = {
    updateDownload: string;
    updateReady: string;
    updateFailed: string;
    showDownload: string;
    appName: string;
    startingTitle: string;
    startingBody: string;
    failedTitle: string;
    failedBody: string;
    retry: string;
    openLogs: string;
    showNola: string;
    restartEngine: string;
    openWorkspace: string;
    openLogFolder: string;
    quit: string;
    file: string;
    edit: string;
    view: string;
    window: string;
    help: string;
    reload: string;
    toggleDevTools: string;
    minimize: string;
    close: string;
    hostCrashed: string;
    backgroundTitle: string;
    backgroundBody: string;
    thirdPartyLicenses: string;
    startAtLogin: string;
    startAtLoginBody: string;
    enabled: string;
    disabled: string;
    packagedOnly: string;
};

const english: DesktopStrings = {
    updateDownload: "Save verified Nola update",
    updateReady: "The verified update has been saved. Finish running tasks before opening the installer.",
    updateFailed: "The update download failed. Check for updates and try again.",
    showDownload: "Show in folder",
    appName: "Nola",
    startingTitle: "Preparing Nola",
    startingBody: "Starting the private local agent and restoring your workspace…",
    failedTitle: "Nola could not start",
    failedBody: "The local agent host did not become ready. Your conversations and model files were not changed.",
    retry: "Try again",
    openLogs: "Open logs",
    showNola: "Show Nola",
    restartEngine: "Restart local agent",
    openWorkspace: "Open workspace",
    openLogFolder: "Open log folder",
    quit: "Quit Nola",
    file: "File",
    edit: "Edit",
    view: "View",
    window: "Window",
    help: "Help",
    reload: "Reload",
    toggleDevTools: "Developer tools",
    minimize: "Minimize",
    close: "Close window",
    hostCrashed: "The local agent stopped unexpectedly. Nola is restarting it…",
    backgroundTitle: "Nola is still running",
    backgroundBody: "Scheduled work and the local agent remain available in the notification area. Use Quit Nola to stop them.",
    thirdPartyLicenses: "Third-party licenses",
    startAtLogin: "Start Nola when I sign in",
    startAtLoginBody: "Keeps scheduled work and the local Agent available in the notification area without opening a window.",
    enabled: "Enabled",
    disabled: "Disabled",
    packagedOnly: "Available after Nola is installed",
};

const simplifiedChinese: DesktopStrings = {
    updateDownload: "保存已校验的 Nola 更新",
    updateReady: "更新包已保存并通过校验。打开安装器前，请先结束运行中的任务。",
    updateFailed: "更新下载失败，请重新检查更新后再试。",
    showDownload: "在文件夹中显示",
    appName: "Nola",
    startingTitle: "正在准备 Nola",
    startingBody: "正在启动本地私有 Agent，并恢复你的工作区…",
    failedTitle: "Nola 无法启动",
    failedBody: "本地 Agent Host 未能就绪。你的对话与模型文件没有被修改。",
    retry: "重新尝试",
    openLogs: "打开日志",
    showNola: "显示 Nola",
    restartEngine: "重启本地 Agent",
    openWorkspace: "打开工作区",
    openLogFolder: "打开日志目录",
    quit: "退出 Nola",
    file: "文件",
    edit: "编辑",
    view: "显示",
    window: "窗口",
    help: "帮助",
    reload: "重新载入",
    toggleDevTools: "开发者工具",
    minimize: "最小化",
    close: "关闭窗口",
    hostCrashed: "本地 Agent 意外停止，Nola 正在自动恢复…",
    backgroundTitle: "Nola 仍在后台运行",
    backgroundBody: "定时任务和本地 Agent 会在通知区域继续运行。选择“退出 Nola”可完全停止。",
    thirdPartyLicenses: "第三方许可证",
    startAtLogin: "登录 Windows 时启动 Nola",
    startAtLoginBody: "无需打开窗口，定时任务和本地 Agent 也能在通知区域保持可用。",
    enabled: "已启用",
    disabled: "未启用",
    packagedOnly: "安装 Nola 后可用",
};

const traditionalChinese: DesktopStrings = {
    updateDownload: "儲存已驗證的 Nola 更新",
    updateReady: "更新套件已儲存並通過驗證。開啟安裝程式前，請先結束執行中的工作。",
    updateFailed: "更新下載失敗，請重新檢查更新後再試。",
    showDownload: "在資料夾中顯示",
    appName: "Nola",
    startingTitle: "正在準備 Nola",
    startingBody: "正在啟動本機私有 Agent，並恢復你的工作區…",
    failedTitle: "Nola 無法啟動",
    failedBody: "本機 Agent Host 未能就緒。你的對話與模型檔案沒有被修改。",
    retry: "重新嘗試",
    openLogs: "開啟日誌",
    showNola: "顯示 Nola",
    restartEngine: "重新啟動本機 Agent",
    openWorkspace: "開啟工作區",
    openLogFolder: "開啟日誌目錄",
    quit: "結束 Nola",
    file: "檔案",
    edit: "編輯",
    view: "檢視",
    window: "視窗",
    help: "說明",
    reload: "重新載入",
    toggleDevTools: "開發者工具",
    minimize: "最小化",
    close: "關閉視窗",
    hostCrashed: "本機 Agent 意外停止，Nola 正在自動復原…",
    backgroundTitle: "Nola 仍在背景執行",
    backgroundBody: "排程工作和本機 Agent 會在通知區域繼續執行。選擇「結束 Nola」可完全停止。",
    thirdPartyLicenses: "第三方授權條款",
    startAtLogin: "登入 Windows 時啟動 Nola",
    startAtLoginBody: "無需開啟視窗，排程工作和本機 Agent 也能在通知區域保持可用。",
    enabled: "已啟用",
    disabled: "未啟用",
    packagedOnly: "安裝 Nola 後可用",
};

const japanese: DesktopStrings = {
    updateDownload: "検証済み Nola 更新を保存",
    updateReady: "検証済み更新を保存しました。インストール前に実行中のタスクを終了してください。",
    updateFailed: "更新をダウンロードできませんでした。再確認してください。",
    showDownload: "フォルダーに表示",
    appName: "Nola",
    startingTitle: "Nola を準備しています",
    startingBody: "ローカルのプライベート Agent を起動し、ワークスペースを復元しています…",
    failedTitle: "Nola を起動できませんでした",
    failedBody: "ローカル Agent Host の準備が完了しませんでした。会話とモデルファイルは変更されていません。",
    retry: "もう一度試す",
    openLogs: "ログを開く",
    showNola: "Nola を表示",
    restartEngine: "ローカル Agent を再起動",
    openWorkspace: "ワークスペースを開く",
    openLogFolder: "ログフォルダを開く",
    quit: "Nola を終了",
    file: "ファイル",
    edit: "編集",
    view: "表示",
    window: "ウィンドウ",
    help: "ヘルプ",
    reload: "再読み込み",
    toggleDevTools: "開発者ツール",
    minimize: "最小化",
    close: "ウィンドウを閉じる",
    hostCrashed: "ローカル Agent が予期せず停止しました。Nola が再起動しています…",
    backgroundTitle: "Nola はバックグラウンドで動作中です",
    backgroundBody: "スケジュールとローカル Agent は通知領域で動作を続けます。「Nola を終了」で完全に停止できます。",
    thirdPartyLicenses: "サードパーティライセンス",
    startAtLogin: "Windows サインイン時に Nola を起動",
    startAtLoginBody: "ウィンドウを開かずに、スケジュールとローカル Agent を通知領域で利用できます。",
    enabled: "有効",
    disabled: "無効",
    packagedOnly: "Nola のインストール後に利用できます",
};

const korean: DesktopStrings = {
    updateDownload: "검증된 Nola 업데이트 저장",
    updateReady: "업데이트가 검증되어 저장되었습니다. 설치 전에 실행 중인 작업을 완료하세요.",
    updateFailed: "업데이트 다운로드에 실패했습니다. 다시 확인하세요.",
    showDownload: "폴더에서 보기",
    appName: "Nola",
    startingTitle: "Nola 준비 중",
    startingBody: "비공개 로컬 Agent를 시작하고 작업 공간을 복원하는 중입니다…",
    failedTitle: "Nola를 시작할 수 없습니다",
    failedBody: "로컬 Agent Host가 준비되지 않았습니다. 대화와 모델 파일은 변경되지 않았습니다.",
    retry: "다시 시도",
    openLogs: "로그 열기",
    showNola: "Nola 보기",
    restartEngine: "로컬 Agent 다시 시작",
    openWorkspace: "작업 공간 열기",
    openLogFolder: "로그 폴더 열기",
    quit: "Nola 종료",
    file: "파일",
    edit: "편집",
    view: "보기",
    window: "창",
    help: "도움말",
    reload: "새로 고침",
    toggleDevTools: "개발자 도구",
    minimize: "최소화",
    close: "창 닫기",
    hostCrashed: "로컬 Agent가 예기치 않게 중지되었습니다. Nola가 다시 시작하는 중입니다…",
    backgroundTitle: "Nola가 백그라운드에서 실행 중입니다",
    backgroundBody: "예약 작업과 로컬 Agent는 알림 영역에서 계속 실행됩니다. 완전히 중지하려면 Nola 종료를 선택하세요.",
    thirdPartyLicenses: "서드파티 라이선스",
    startAtLogin: "Windows 로그인 시 Nola 시작",
    startAtLoginBody: "창을 열지 않아도 예약 작업과 로컬 Agent를 알림 영역에서 계속 사용할 수 있습니다.",
    enabled: "사용",
    disabled: "사용 안 함",
    packagedOnly: "Nola 설치 후 사용할 수 있습니다",
};

const german: DesktopStrings = {
    updateDownload: "Geprüftes Nola-Update speichern",
    updateReady: "Das geprüfte Update wurde gespeichert. Beenden Sie laufende Aufgaben, bevor Sie den Installer öffnen.",
    updateFailed: "Der Update-Download ist fehlgeschlagen. Prüfen Sie erneut auf Updates.",
    showDownload: "Im Ordner anzeigen",
    appName: "Nola",
    startingTitle: "Nola wird vorbereitet",
    startingBody: "Privater lokaler Agent wird gestartet und der Arbeitsbereich wird wiederhergestellt…",
    failedTitle: "Nola konnte nicht gestartet werden",
    failedBody: "Der lokale Agent Host ist nicht bereit geworden. Ihre Gespräche und Modelldateien wurden nicht verändert.",
    retry: "Erneut versuchen",
    openLogs: "Logs öffnen",
    showNola: "Nola anzeigen",
    restartEngine: "Lokalen Agent neu starten",
    openWorkspace: "Arbeitsbereich öffnen",
    openLogFolder: "Log-Ordner öffnen",
    quit: "Nola beenden",
    file: "Datei",
    edit: "Bearbeiten",
    view: "Ansicht",
    window: "Fenster",
    help: "Hilfe",
    reload: "Neu laden",
    toggleDevTools: "Entwicklertools",
    minimize: "Minimieren",
    close: "Fenster schließen",
    hostCrashed: "Der lokale Agent wurde unerwartet beendet. Nola startet ihn neu…",
    backgroundTitle: "Nola wird weiterhin ausgeführt",
    backgroundBody: "Geplante Aufgaben und der lokale Agent bleiben im Infobereich verfügbar. Zum vollständigen Beenden „Nola beenden“ wählen.",
    thirdPartyLicenses: "Drittanbieterlizenzen",
    startAtLogin: "Nola bei der Anmeldung starten",
    startAtLoginBody: "Hält geplante Aufgaben und den lokalen Agent im Infobereich bereit, ohne ein Fenster zu öffnen.",
    enabled: "Aktiviert",
    disabled: "Deaktiviert",
    packagedOnly: "Nach der Installation von Nola verfügbar",
};

const spanish: DesktopStrings = {
    updateDownload: "Guardar actualización verificada de Nola",
    updateReady: "La actualización verificada se ha guardado. Finaliza las tareas en curso antes de abrir el instalador.",
    updateFailed: "Error al descargar la actualización. Vuelve a comprobar si hay actualizaciones.",
    showDownload: "Mostrar en la carpeta",
    appName: "Nola",
    startingTitle: "Preparando Nola",
    startingBody: "Iniciando el agente local privado y restaurando tu espacio de trabajo…",
    failedTitle: "Nola no pudo iniciarse",
    failedBody: "El Agent Host local no quedó listo. Tus conversaciones y archivos de modelo no se han modificado.",
    retry: "Reintentar",
    openLogs: "Abrir registros",
    showNola: "Mostrar Nola",
    restartEngine: "Reiniciar agente local",
    openWorkspace: "Abrir espacio de trabajo",
    openLogFolder: "Abrir carpeta de registros",
    quit: "Salir de Nola",
    file: "Archivo",
    edit: "Editar",
    view: "Ver",
    window: "Ventana",
    help: "Ayuda",
    reload: "Recargar",
    toggleDevTools: "Herramientas de desarrollo",
    minimize: "Minimizar",
    close: "Cerrar ventana",
    hostCrashed: "El agente local se detuvo inesperadamente. Nola lo está reiniciando…",
    backgroundTitle: "Nola sigue ejecutándose",
    backgroundBody: "Las tareas programadas y el agente local siguen disponibles en el área de notificaciones. Usa Salir de Nola para detenerlos.",
    thirdPartyLicenses: "Licencias de terceros",
    startAtLogin: "Iniciar Nola al iniciar sesión",
    startAtLoginBody: "Mantiene disponibles las tareas programadas y el agente local en el área de notificaciones sin abrir ninguna ventana.",
    enabled: "Activado",
    disabled: "Desactivado",
    packagedOnly: "Disponible después de instalar Nola",
};

const french: DesktopStrings = {
    updateDownload: "Enregistrer la mise à jour vérifiée de Nola",
    updateReady: "La mise à jour vérifiée a été enregistrée. Terminez les tâches en cours avant d’ouvrir le programme d’installation.",
    updateFailed: "Échec du téléchargement de la mise à jour. Vérifiez de nouveau les mises à jour.",
    showDownload: "Afficher dans le dossier",
    appName: "Nola",
    startingTitle: "Préparation de Nola",
    startingBody: "Démarrage de l’agent local privé et restauration de votre espace de travail…",
    failedTitle: "Nola n’a pas pu démarrer",
    failedBody: "L’Agent Host local n’est pas devenu prêt. Vos conversations et fichiers de modèle n’ont pas été modifiés.",
    retry: "Réessayer",
    openLogs: "Ouvrir les journaux",
    showNola: "Afficher Nola",
    restartEngine: "Redémarrer l’agent local",
    openWorkspace: "Ouvrir l’espace de travail",
    openLogFolder: "Ouvrir le dossier des journaux",
    quit: "Quitter Nola",
    file: "Fichier",
    edit: "Édition",
    view: "Affichage",
    window: "Fenêtre",
    help: "Aide",
    reload: "Recharger",
    toggleDevTools: "Outils de développement",
    minimize: "Réduire",
    close: "Fermer la fenêtre",
    hostCrashed: "L’agent local s’est arrêté de façon inattendue. Nola le redémarre…",
    backgroundTitle: "Nola fonctionne toujours en arrière-plan",
    backgroundBody: "Les tâches planifiées et l’agent local restent disponibles dans la zone de notification. Choisissez Quitter Nola pour les arrêter.",
    thirdPartyLicenses: "Licences tierces",
    startAtLogin: "Lancer Nola à la connexion",
    startAtLoginBody: "Maintient les tâches planifiées et l’agent local disponibles dans la zone de notification sans ouvrir de fenêtre.",
    enabled: "Activé",
    disabled: "Désactivé",
    packagedOnly: "Disponible après l’installation de Nola",
};

const russian: DesktopStrings = {
    updateDownload: "Сохранить проверенное обновление Nola",
    updateReady: "Проверенное обновление сохранено. Завершите текущие задачи перед запуском установщика.",
    updateFailed: "Не удалось загрузить обновление. Проверьте обновления и повторите попытку.",
    showDownload: "Показать в папке",
    appName: "Nola",
    startingTitle: "Подготовка Nola",
    startingBody: "Запуск локального приватного агента и восстановление рабочей области…",
    failedTitle: "Не удалось запустить Nola",
    failedBody: "Локальный Agent Host не перешёл в состояние готовности. Ваши диалоги и файлы моделей не были изменены.",
    retry: "Повторить",
    openLogs: "Открыть журналы",
    showNola: "Показать Nola",
    restartEngine: "Перезапустить локального агента",
    openWorkspace: "Открыть рабочую область",
    openLogFolder: "Открыть папку журналов",
    quit: "Выйти из Nola",
    file: "Файл",
    edit: "Правка",
    view: "Вид",
    window: "Окно",
    help: "Справка",
    reload: "Перезагрузить",
    toggleDevTools: "Инструменты разработчика",
    minimize: "Свернуть",
    close: "Закрыть окно",
    hostCrashed: "Локальный агент неожиданно остановился. Nola перезапускает его…",
    backgroundTitle: "Nola продолжает работать в фоне",
    backgroundBody: "Запланированные задачи и локальный агент остаются доступными в области уведомлений. Выберите «Выйти из Nola» для остановки.",
    thirdPartyLicenses: "Сторонние лицензии",
    startAtLogin: "Запускать Nola при входе",
    startAtLoginBody: "Сохраняет доступность запланированных задач и локального агента в области уведомлений без открытия окна.",
    enabled: "Включено",
    disabled: "Отключено",
    packagedOnly: "Доступно после установки Nola",
};

export function stringsFor(locale: string): DesktopStrings {
    const normalized = locale.toLowerCase().replace("_", "-");
    if (normalized.startsWith("zh-hant") || normalized.startsWith("zh-tw") || normalized.startsWith("zh-hk") || normalized.startsWith("zh-mo")) {
        return traditionalChinese;
    }
    if (normalized.startsWith("zh")) return simplifiedChinese;
    if (normalized.startsWith("ja")) return japanese;
    if (normalized.startsWith("ko")) return korean;
    if (normalized.startsWith("de")) return german;
    if (normalized.startsWith("es")) return spanish;
    if (normalized.startsWith("fr")) return french;
    if (normalized.startsWith("ru")) return russian;
    return english;
}
