import type { Locale } from '../i18n';

const languages: Locale[] = ['zh-Hans', 'en', 'de', 'es', 'fr', 'ja', 'ru'];
const entries: Record<string, readonly [string, string, string, string, string, string, string]> = {
    title: ['会话数据源', 'Conversation data sources', 'Datenquellen der Unterhaltung', 'Fuentes de la conversación', 'Sources de la conversation', '会話のデータソース', 'Источники данных диалога'],
    scope: ['只改变本会话；全局阻止和读取权限仍然有效。', 'Changes apply only to this conversation; global blocks and read permissions still apply.', 'Änderungen gelten nur für diese Unterhaltung; globale Sperren und Leserechte bleiben bestehen.', 'Los cambios solo afectan a esta conversación; los bloqueos globales y permisos de lectura siguen vigentes.', 'Les changements ne concernent que cette conversation ; les blocages globaux et droits de lecture restent applicables.', '変更はこの会話だけに適用されます。全体のブロックと読み取り権限は維持されます。', 'Изменения касаются только этого диалога; глобальные запреты и права чтения сохраняются.'],
    refresh: ['刷新', 'Refresh', 'Aktualisieren', 'Actualizar', 'Actualiser', '更新', 'Обновить'],
    loading: ['正在读取数据源…', 'Loading data sources…', 'Datenquellen werden geladen…', 'Cargando fuentes…', 'Chargement des sources…', 'データソースを読み込み中…', 'Загрузка источников…'],
    empty: ['还没有可用的数据源。可在数据中心添加并验证。', 'No data sources yet. Add and verify one in Data Center.', 'Noch keine Datenquellen. Im Datenzentrum hinzufügen und prüfen.', 'Aún no hay fuentes. Añádelas y verifícalas en el Centro de datos.', 'Aucune source pour le moment. Ajoutez-en et vérifiez-les dans le Centre de données.', 'データソースはまだありません。データセンターで追加・検証できます。', 'Источников пока нет. Добавьте и проверьте их в центре данных.'],
    manage: ['打开数据中心', 'Open Data Center', 'Datenzentrum öffnen', 'Abrir Centro de datos', 'Ouvrir le Centre de données', 'データセンターを開く', 'Открыть центр данных'],
    inherit: ['继承全局策略', 'Inherit global policy', 'Globale Richtlinie übernehmen', 'Heredar política global', 'Hériter de la règle globale', '全体設定を継承', 'Наследовать общую политику'],
    enable: ['本会话启用', 'Enable for this conversation', 'Für diese Unterhaltung aktivieren', 'Activar para esta conversación', 'Activer pour cette conversation', 'この会話で有効', 'Включить для этого диалога'],
    disable: ['本会话关闭', 'Disable for this conversation', 'Für diese Unterhaltung deaktivieren', 'Desactivar para esta conversación', 'Désactiver pour cette conversation', 'この会話で無効', 'Отключить для этого диалога'],
    saving: ['正在保存…', 'Saving…', 'Wird gespeichert…', 'Guardando…', 'Enregistrement…', '保存中…', 'Сохранение…'],
    loadFailed: ['无法读取会话数据源；请重试。', 'Could not load conversation data sources. Try again.', 'Datenquellen konnten nicht geladen werden. Bitte erneut versuchen.', 'No se pudieron cargar las fuentes. Inténtalo de nuevo.', 'Impossible de charger les sources. Réessayez.', '会話のデータソースを読み込めません。再試行してください。', 'Не удалось загрузить источники. Повторите попытку.'],
    saveFailed: ['未能保存；已重新读取服务器状态。', 'Save failed; the server state was reloaded.', 'Speichern fehlgeschlagen; Serverstatus wurde neu geladen.', 'Error al guardar; se recargó el estado del servidor.', 'Échec de l’enregistrement ; l’état du serveur a été rechargé.', '保存に失敗したため、サーバーの状態を再読み込みしました。', 'Сохранение не удалось; состояние сервера загружено повторно.'],
    invalidResponse: ['服务器返回的数据源状态无效。', 'The server returned an invalid data-source state.', 'Der Server lieferte einen ungültigen Datenquellenstatus.', 'El servidor devolvió un estado de fuente no válido.', 'Le serveur a renvoyé un état de source invalide.', 'サーバーから無効なデータソース状態が返されました。', 'Сервер вернул недопустимое состояние источника.'],
    ready: ['可读取', 'Ready to read', 'Lesebereit', 'Lista para leer', 'Prête à lire', '読み取り可能', 'Готов к чтению'],
    disabled_for_conversation: ['本会话已关闭', 'Disabled for this conversation', 'Für diese Unterhaltung deaktiviert', 'Desactivada para esta conversación', 'Désactivée pour cette conversation', 'この会話で無効', 'Отключён для этого диалога'],
    conversation_opt_in_required: ['等待本会话启用', 'Enable for this conversation first', 'Zuerst für diese Unterhaltung aktivieren', 'Actívala para esta conversación', 'À activer pour cette conversation', 'この会話での有効化が必要', 'Сначала включите для этого диалога'],
    consent_required: ['需要本会话明确同意', 'Explicit conversation consent required', 'Ausdrückliche Zustimmung erforderlich', 'Se requiere consentimiento explícito', 'Consentement explicite requis', 'この会話での明示的な同意が必要', 'Требуется явное согласие'],
    blocked: ['全局已阻止', 'Globally blocked', 'Global gesperrt', 'Bloqueada globalmente', 'Bloquée globalement', '全体設定でブロック中', 'Глобально заблокирован'],
    contract_missing: ['需要验证读取方式', 'Read contract missing', 'Lesevertrag fehlt', 'Falta el contrato de lectura', 'Contrat de lecture manquant', '読み取り契約がありません', 'Нет контракта чтения'],
    contract_invalid: ['读取契约无效', 'Read contract invalid', 'Lesevertrag ungültig', 'Contrato de lectura no válido', 'Contrat de lecture invalide', '読み取り契約が無効です', 'Контракт чтения недействителен'],
    validation_failed: ['验证失败', 'Validation failed', 'Prüfung fehlgeschlagen', 'Validación fallida', 'Échec de la vérification', '検証に失敗', 'Проверка не пройдена'],
    stale: ['需要重新验证', 'Needs revalidation', 'Erneute Prüfung erforderlich', 'Requiere nueva validación', 'Nouvelle vérification requise', '再検証が必要', 'Нужна повторная проверка'],
};

export const chatDataSourceMessages: Record<Locale, Record<string, string>> = Object.fromEntries(languages.map((language, index) => [language,
    Object.fromEntries(Object.entries(entries).map(([key, values]) => [`chatDataSources.${key}`, values[index]])),
])) as Record<Locale, Record<string, string>>;
