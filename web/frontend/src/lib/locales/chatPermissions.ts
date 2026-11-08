import type { Locale } from '../i18n';

const keys = ['title', 'noDetails', 'scope', 'inactive', 'ready', 'submitting', 'accepted', 'unknown', 'rejected', 'expired', 'invalid', 'deny', 'return', 'allowConversation', 'allowOnce', 'guidance', 'guidanceHint', 'saveGuidance', 'instructionLimit', 'retrySame', 'refresh'] as const;
const translations: Record<Locale, string[]> = {
    'zh-Hans': [
        '工具权限确认', '缺少可安全显示的调用详情，暂不能批准。', '决定仅针对当前工具请求。最终权限和执行结果由 Core 确认。',
        '此请求已不再等待确认。', '工具已暂停，等待你的决定。', '正在提交决定…', 'Host 已接收决定；等待执行状态更新，并不代表操作已完成。',
        '提交结果未知。请刷新状态，或仅重试完全相同的决定。', 'Host 未接受决定，请刷新请求状态。', '确认已过期，请刷新请求。', '权限请求无效或协议不受支持，不能批准。',
        '拒绝', '返回输入框', '本会话允许', '仅允许本次', '补充说明', '说明不会修改原工具参数，也不代表批准；提交后等待新的确认。', '提交说明',
        '说明不能为空，每条最多 4 KiB；累计最多 16 条、16 KiB。', '重试相同决定', '刷新请求状态',
    ],
    en: [
        'Tool permission', 'Safe call details are unavailable. Approval is disabled.', 'This decision applies to this tool request. Core confirms final authorization and execution.',
        'This request is no longer awaiting a decision.', 'The tool is paused for your decision.', 'Submitting decision…', 'Host received the decision. Awaiting execution updates; this does not mean the operation completed.',
        'Submission outcome is unknown. Refresh status or retry only the exact same decision.', 'Host did not accept the decision. Refresh request status.', 'This challenge expired. Refresh the request.', 'Invalid or unsupported permission request. Approval is disabled.',
        'Deny', 'Return to composer', 'Allow for conversation', 'Allow once', 'Guidance', 'Guidance does not change the original arguments or grant approval. Await a new challenge after submitting.', 'Submit guidance',
        'Guidance must be nonempty: up to 4 KiB each, 16 entries and 16 KiB total.', 'Retry same decision', 'Refresh request status',
    ],
    de: [
        'Werkzeugberechtigung', 'Sicher anzeigbare Aufrufdetails fehlen. Genehmigung deaktiviert.', 'Die Entscheidung gilt für diese Werkzeuganfrage. Core bestätigt Berechtigung und Ausführung.',
        'Diese Anfrage wartet nicht mehr auf eine Entscheidung.', 'Das Werkzeug wartet auf Ihre Entscheidung.', 'Entscheidung wird übermittelt…', 'Host hat die Entscheidung erhalten. Ausführungsstatus steht aus; der Vorgang ist damit nicht abgeschlossen.',
        'Übermittlungsergebnis unbekannt. Status aktualisieren oder exakt dieselbe Entscheidung erneut senden.', 'Host hat die Entscheidung nicht angenommen. Anfragestatus aktualisieren.', 'Die Anfrage ist abgelaufen. Bitte aktualisieren.', 'Ungültige oder nicht unterstützte Berechtigungsanfrage. Genehmigung deaktiviert.',
        'Ablehnen', 'Zurück zur Eingabe', 'Für dieses Gespräch erlauben', 'Einmal erlauben', 'Hinweis', 'Hinweise ändern weder die ursprünglichen Argumente noch erteilen sie eine Genehmigung. Danach auf eine neue Anfrage warten.', 'Hinweis senden',
        'Hinweis darf nicht leer sein: höchstens 4 KiB je Eintrag, 16 Einträge und insgesamt 16 KiB.', 'Dieselbe Entscheidung erneut senden', 'Anfragestatus aktualisieren',
    ],
    es: [
        'Permiso de herramienta', 'No hay detalles seguros de la llamada. Aprobación desactivada.', 'La decisión corresponde a esta solicitud. Core confirma la autorización y ejecución finales.',
        'Esta solicitud ya no espera una decisión.', 'La herramienta está en pausa esperando tu decisión.', 'Enviando decisión…', 'El Host recibió la decisión. Esperando el estado de ejecución; no significa que la operación haya terminado.',
        'Resultado del envío desconocido. Actualiza el estado o reintenta exactamente la misma decisión.', 'El Host no aceptó la decisión. Actualiza el estado.', 'La solicitud caducó. Actualízala.', 'Solicitud de permiso inválida o no compatible. Aprobación desactivada.',
        'Denegar', 'Volver al editor', 'Permitir en esta conversación', 'Permitir una vez', 'Indicaciones', 'Las indicaciones no cambian los argumentos originales ni autorizan la operación. Tras enviarlas, espera una nueva solicitud.', 'Enviar indicaciones',
        'Las indicaciones no pueden estar vacías: máximo 4 KiB por entrada, 16 entradas y 16 KiB en total.', 'Reintentar la misma decisión', 'Actualizar estado',
    ],
    fr: [
        'Autorisation de l’outil', 'Les détails sûrs de l’appel sont indisponibles. Approbation désactivée.', 'Cette décision concerne cette demande. Core confirme l’autorisation et l’exécution finales.',
        'Cette demande n’attend plus de décision.', 'L’outil est en pause en attendant votre décision.', 'Envoi de la décision…', 'L’hôte a reçu la décision. En attente du statut d’exécution ; cela ne signifie pas que l’opération est terminée.',
        'Résultat de l’envoi inconnu. Actualisez le statut ou renvoyez exactement la même décision.', 'L’hôte n’a pas accepté la décision. Actualisez le statut.', 'La demande a expiré. Actualisez-la.', 'Demande d’autorisation invalide ou non prise en charge. Approbation désactivée.',
        'Refuser', 'Retour à la saisie', 'Autoriser pour cette conversation', 'Autoriser une fois', 'Instructions', 'Les instructions ne modifient pas les arguments d’origine et ne donnent pas d’autorisation. Attendez ensuite une nouvelle demande.', 'Envoyer les instructions',
        'Les instructions ne doivent pas être vides : 4 Kio par entrée, 16 entrées et 16 Kio au total au maximum.', 'Renvoyer la même décision', 'Actualiser le statut',
    ],
    ja: [
        'ツールの権限確認', '安全に表示できる呼び出し詳細がありません。承認できません。', 'この判断は現在のツール要求に適用されます。最終的な権限と実行結果は Core が確認します。',
        'この要求は確認待ちではありません。', 'ツールは判断を待って停止しています。', '判断を送信中…', 'Host が判断を受信しました。実行状態の更新を待っています。操作の完了を意味しません。',
        '送信結果が不明です。状態を更新するか、まったく同じ判断のみ再送してください。', 'Host は判断を受け付けませんでした。状態を更新してください。', '確認期限が切れました。要求を更新してください。', '権限要求が無効、または未対応です。承認できません。',
        '拒否', '入力欄に戻る', 'この会話で許可', '今回のみ許可', '補足指示', '指示は元の引数を変更せず、承認にもなりません。送信後は新しい確認を待ってください。', '指示を送信',
        '空の指示は送信できません。各 4 KiB、合計 16 件・16 KiB までです。', '同じ判断を再送', '要求の状態を更新',
    ],
    ru: [
        'Разрешение инструмента', 'Безопасные сведения о вызове недоступны. Одобрение отключено.', 'Решение относится к этому запросу. Core подтверждает итоговое разрешение и выполнение.',
        'Этот запрос больше не ожидает решения.', 'Инструмент приостановлен и ожидает вашего решения.', 'Отправка решения…', 'Host получил решение. Ожидается статус выполнения; это не означает завершения операции.',
        'Результат отправки неизвестен. Обновите статус или повторите строго то же решение.', 'Host не принял решение. Обновите статус запроса.', 'Срок запроса истёк. Обновите запрос.', 'Недопустимый или неподдерживаемый запрос разрешения. Одобрение отключено.',
        'Отклонить', 'Вернуться к вводу', 'Разрешить для беседы', 'Разрешить один раз', 'Указания', 'Указания не меняют исходные аргументы и не дают разрешения. После отправки ожидайте нового запроса.', 'Отправить указания',
        'Указания не могут быть пустыми: до 4 КиБ на запись, 16 записей и 16 КиБ суммарно.', 'Повторить то же решение', 'Обновить статус запроса',
    ],
};
export const chatPermissionMessages: Record<Locale, Record<string, string>> = Object.fromEntries(
    Object.entries(translations).map(([language, values]) => {
        if (values.length !== keys.length) throw new Error(`Incomplete permission catalog: ${language}`);
        return [language, Object.fromEntries(keys.map((key, index) => [`chatPermission.${key}`, values[index]]))];
    }),
) as Record<Locale, Record<string, string>>;
