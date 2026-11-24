// SPDX-License-Identifier: Apache-2.0
import type { Locale } from '../i18n.ts';

const languages: Locale[] = ['zh-Hans', 'en', 'de', 'es', 'fr', 'ja', 'ru'];
const entries: Record<string, readonly string[]> = {
    buttonLabel: ['工作区变更', 'Workspace changes', 'Arbeitsbereich-Änderungen', 'Cambios del espacio', 'Modifications de l’espace', 'ワークスペースの変更', 'Изменения рабочей области'],
    panelTitle: ['工作区工作流', 'Workspace workflow', 'Arbeitsbereich-Workflow', 'Flujo del espacio', 'Flux de travail', 'ワークスペースワークフロー', 'Рабочий процесс'],
    planMode: ['Plan 只读', 'Plan', 'Plan', 'Plan', 'Plan', 'Plan', 'План'],
    buildMode: ['Build 构建', 'Build', 'Build', 'Build', 'Build', 'Build', 'Сборка'],
    planModeHint: ['Plan 模式（只读）：禁止写入文件、执行修改命令和 Git 变更', 'Plan mode (read-only): file edits, mutating shell commands, and git writes are blocked', 'Plan-Modus (schreibgeschützt): Dateiänderungen, Schreibbefehle und Git-Schreibvorgänge sind blockiert', 'Modo Plan (solo lectura): se bloquean ediciones de archivos, comandos con cambios y escrituras Git', 'Mode Plan (lecture seule) : modifications de fichiers, commandes destructives et écritures Git bloquées', 'Plan モード（読み取り専用）：ファイル編集・変更コマンド・Git 書き込みをブロックします', 'Режим плана (только чтение): изменение файлов, команд и записи в Git заблокированы'],
    buildModeHint: ['Build 模式：允许按权限策略执行代码修改与工具调用', 'Build mode: file edits and tool mutations follow active permission rules', 'Build-Modus: Dateiänderungen und Werkzeuge folgen den aktiven Berechtigungsregeln', 'Modo Build: las ediciones y herramientas siguen las reglas de permisos activas', 'Mode Build : les modifications et outils suivent les règles d’autorisation actives', 'Build モード：権限ルールに従って編集やツール実行を許可します', 'Режим сборки: изменения файлов и инструменты работают по правилам разрешений'],
    tabDiff: ['差异', 'Diff', 'Diff', 'Diff', 'Diff', '差分', 'Diff'],
    tabCheckpoints: ['检查点', 'Checkpoints', 'Prüfpunkte', 'Puntos de control', 'Points de contrôle', 'チェックポイント', 'Контрольные точки'],
    tabWorktrees: ['Worktrees', 'Worktrees', 'Worktrees', 'Worktrees', 'Worktrees', 'Worktrees', 'Worktrees'],
    undoTurn: ['撤销轮次', 'Undo Turn', 'Zug rückgängig', 'Deshacer turno', 'Annuler le tour', 'ターンを戻す', 'Отменить шаг'],
    redoTurn: ['重做', 'Redo', 'Wiederholen', 'Rehacer', 'Rétablir', 'やり直す', 'Повторить'],
    allFiles: ['全部文件', 'All files', 'Alle Dateien', 'Todos los archivos', 'Tous les fichiers', 'すべてのファイル', 'Все файлы'],
    cleanTree: ['工作区干净，暂无未提交改动。', 'Working tree is clean (no uncommitted changes).', 'Arbeitsverzeichnis ist sauber (keine ungespeicherten Änderungen).', 'El árbol de trabajo está limpio (sin cambios sin confirmar).', 'L’arbre de travail est propre (aucune modification non validée).', '作業ツリーはクリーンです（未コミットの変更はありません）。', 'Рабочее дерево чистое (нет незафиксированных изменений).'],
    notGitRepo: ['当前工作区不是 Git 仓库。', 'Current workspace is not a Git repository.', 'Der aktuelle Arbeitsbereich ist kein Git-Repository.', 'El espacio de trabajo actual no es un repositorio Git.', 'L’espace de travail actuel n’est pas un dépôt Git.', '現在のワークスペースは Git リポジトリではありません。', 'Текущая рабочая область не является Git-репозиторием.'],
    diffTruncated: ['差异内容已截断至 64 KB，可点击单个文件查看聚焦差异。', 'Diff output truncated at 64 KB. Select a file to view its focused diff.', 'Diff-Ausgabe bei 64 KB gekürzt. Wähle eine Datei für eine fokussierte Ansicht.', 'La salida del diff se truncó a 64 KB. Selecciona un archivo para ver su diff.', 'Sortie diff tronquée à 64 Ko. Sélectionnez un fichier pour voir son diff ciblé.', '差分出力は 64 KB で省略されました。個別ファイルを選択すると詳細を確認できます。', 'Вывод diff обрезан до 64 КБ. Выберите файл для подробного просмотра.'],
    noDiffText: ['暂无可展示的文本差异（可能仅包含未跟踪文件，点击文件可预览内容）。', 'No unified diff text for tracked files. Select an untracked file to inspect its contents.', 'Kein Diff-Text für verfolgte Dateien. Wähle eine unverfolgte Datei zur Vorschau.', 'No hay texto diff para archivos rastreados. Selecciona un archivo sin seguimiento para verlo.', 'Aucun texte diff pour les fichiers suivis. Sélectionnez un fichier non suivi pour l’inspecter.', '追跡ファイルの差分はありません。未追跡ファイルを選択すると内容を確認できます。', 'Нет текста diff для отслеживаемых файлов. Выберите неотслеживаемый файл для просмотра.'],
    noCheckpoints: ['当前会话尚未记录轮次检查点。在 Build 模式下发送消息前会自动创建快照。', 'No turn checkpoints captured yet. Snapshots are recorded automatically before Build turns.', 'Noch keine Prüfpunkte erfasst. Schnappschüsse werden vor Build-Zügen automatisch erstellt.', 'Aún no hay puntos de control. Las instantáneas se crean automáticamente antes de los turnos Build.', 'Aucun point de contrôle enregistré. Les instantanés sont créés avant chaque tour Build.', 'チェックポイントはまだありません。Build モードの実行前に自動作成されます。', 'Контрольные точки ещё не созданы. Снимки создаются автоматически перед шагами Build.'],
    turnLabel: ['轮次 #{turn}', 'Turn #{turn}', 'Zug #{turn}', 'Turno #{turn}', 'Tour #{turn}', 'ターン #{turn}', 'Шаг #{turn}'],
    undoSuccess: ['已撤销至轮次 #{turn}：恢复 {restored} 个文件，移除 {removed} 个文件', 'Undid to turn #{turn}: restored {restored}, removed {removed}', 'Auf Zug #{turn} zurückgesetzt: {restored} wiederhergestellt, {removed} entfernt', 'Deshecho al turno #{turn}: {restored} restaurados, {removed} eliminados', 'Annulé au tour #{turn} : {restored} restaurés, {removed} supprimés', 'ターン #{turn} に戻しました：復元 {restored}、削除 {removed}', 'Откат к шагу #{turn}: восстановлено {restored}, удалено {removed}'],
    redoSuccess: ['已重做轮次 #{turn}：恢复 {restored} 个文件，移除 {removed} 个文件', 'Redid turn #{turn}: restored {restored}, removed {removed}', 'Zug #{turn} wiederholt: {restored} wiederhergestellt, {removed} entfernt', 'Rehecho el turno #{turn}: {restored} restaurados, {removed} eliminados', 'Rétabli le tour #{turn} : {restored} restaurés, {removed} supprimés', 'ターン #{turn} をやり直しました：復元 {restored}、削除 {removed}', 'Повтор шага #{turn}: восстановлено {restored}, удалено {removed}'],
    newWorktree: ['新建 Worktree', 'New Worktree', 'Neues Worktree', 'Nuevo Worktree', 'Nouveau Worktree', '新規 Worktree', 'Новый Worktree'],
    branchPlaceholder: ['分支名称，例如 feat/diff-viewer', 'Branch name, e.g. feat/diff-viewer', 'Branch-Name, z. B. feat/diff-viewer', 'Nombre de rama, ej. feat/diff-viewer', 'Nom de branche, ex. feat/diff-viewer', 'ブランチ名（例: feat/diff-viewer）', 'Имя ветки, напр. feat/diff-viewer'],
    createWorktree: ['创建', 'Create', 'Erstellen', 'Crear', 'Créer', '作成', 'Создать'],
    creatingWorktree: ['创建中…', 'Creating…', 'Wird erstellt…', 'Creando…', 'Création…', '作成中…', 'Создание…'],
    worktreeCreated: ['已创建 Worktree 分支 {branch}', 'Created worktree for {branch}', 'Worktree für {branch} erstellt', 'Worktree creado para {branch}', 'Worktree créé pour {branch}', 'Worktree（{branch}）を作成しました', 'Создан worktree для {branch}'],
    noWorktrees: ['未找到 Git Worktree。', 'No Git worktrees found.', 'Keine Git-Worktrees gefunden.', 'No se encontraron worktrees de Git.', 'Aucun worktree Git trouvé.', 'Git worktree が見つかりません。', 'Git worktree не найдены.'],
    badgeMain: ['主仓库', 'Main', 'Haupt', 'Principal', 'Principal', 'メイン', 'Основной'],
    badgeCurrent: ['当前', 'Current', 'Aktuell', 'Actual', 'Actuel', '現在', 'Текущий'],
    refresh: ['刷新', 'Refresh', 'Aktualisieren', 'Actualizar', 'Actualiser', '更新', 'Обновить'],
    close: ['关闭', 'Close', 'Schließen', 'Cerrar', 'Fermer', '閉じる', 'Закрыть'],
    invalidBranch: ['分支名仅支持字母、数字及 - _ / .，且不能以 - / . 开头', 'Branch name may only use letters, digits, -, _, /, . and cannot start with -, /, or .', 'Branch-Name darf nur Buchstaben, Ziffern, -, _, /, . enthalten und nicht mit -, / oder . beginnen', 'La rama solo admite letras, dígitos, -, _, /, . y no puede empezar por -, / ni .', 'Le nom de branche accepte uniquement lettres, chiffres, -, _, /, . sans débuter par -, / ou .', 'ブランチ名には英数字と - _ / . のみ使用でき、- / . で始められません', 'Имя ветки может содержать только буквы, цифры, -, _, /, . и не начинаться с -, / или .'],
    loading: ['正在加载工作区状态…', 'Loading workspace state…', 'Arbeitsbereichsstatus wird geladen…', 'Cargando estado del espacio…', 'Chargement de l’espace…', 'ワークスペースを読み込み中…', 'Загрузка состояния рабочей области…'],
};

export const workspaceWorkflowMessages = Object.fromEntries(languages.map((language, index) => [language,
    Object.fromEntries(Object.entries(entries).map(([key, values]) => [`workspaceWorkflow.${key}`, values[index]])),
]));
