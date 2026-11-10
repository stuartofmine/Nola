import type { Locale } from '../i18n';
const languages: Locale[] = ['zh-Hans','en','de','es','fr','ja','ru'];
const entries: Record<string,readonly string[]> = {
    "local": [
        "工具执行位置：本机；使用此设备可用的工具与文件。",
        "Tools run locally, using this device’s available tools and files.",
        "Werkzeuge laufen lokal und verwenden die Werkzeuge und Dateien dieses Geräts.",
        "Las herramientas se ejecutan localmente con los recursos y archivos de este dispositivo.",
        "Les outils s’exécutent localement avec les ressources et fichiers de cet appareil.",
        "ツールはこの端末で実行され、この端末の利用可能なツールとファイルを使用します。",
        "Инструменты выполняются локально, используя доступные ресурсы и файлы этого устройства."
    ],
    "host": [
        "工具与文件来自所选 Host；连接不会自动授予本机权限。",
        "Tools and files belong to the selected Host; connecting does not grant local permissions.",
        "Werkzeuge und Dateien gehören zum ausgewählten Host; eine Verbindung erteilt keine lokalen Berechtigungen.",
        "Las herramientas y archivos pertenecen al Host seleccionado; conectarse no concede permisos locales.",
        "Les outils et fichiers appartiennent au Host sélectionné ; la connexion n’accorde pas de droits locaux.",
        "ツールとファイルは選択した Host のものです。接続しても端末の権限は付与されません。",
        "Инструменты и файлы находятся на выбранном Host; подключение не даёт локальных разрешений."
    ],
    "child": [
        "子任务上下文独立；能力不超出委派时的授权范围。",
        "Child tasks have separate context and stay within their delegated permissions.",
        "Unteraufgaben haben eigenen Kontext und bleiben innerhalb der delegierten Berechtigungen.",
        "Las subtareas tienen contexto propio y respetan los permisos delegados.",
        "Les sous-tâches ont leur propre contexte et respectent les autorisations déléguées.",
        "子タスクのコンテキストは独立し、委任された権限の範囲内で実行されます。",
        "У подзадач отдельный контекст; их полномочия ограничены делегированными разрешениями."
    ],
    "noApprovalBridge": [
        "没有 Nola 审批桥；原生权限可能拒绝需确认的操作。",
        "No Nola approval bridge; native permissions may deny operations requiring confirmation.",
        "Keine Nola-Freigabebrücke; native Berechtigungen können bestätigungspflichtige Vorgänge ablehnen.",
        "Sin puente de aprobación de Nola; los permisos nativos pueden denegar acciones que requieran confirmación.",
        "Pas de passerelle d’approbation Nola ; les droits natifs peuvent refuser les actions nécessitant une confirmation.",
        "Nola の承認連携はありません。確認が必要な操作は実行側の権限により拒否される場合があります。",
        "Нет моста согласований Nola; собственные правила исполнителя могут запретить действия, требующие подтверждения."
    ]
};
export const executionScopeMessages = Object.fromEntries(languages.map((language,index)=>[language,Object.fromEntries(Object.entries(entries).map(([key,values])=>['execution.scope.'+key,values[index]]))]));
