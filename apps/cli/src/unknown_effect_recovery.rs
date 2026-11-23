// SPDX-License-Identifier: Apache-2.0
//! Explicit user recovery controls. This module never creates or dispatches a
//! Run, and is deliberately absent from the model's tool catalog.
use crate::{application_port::CliApplicationPort, config::Config};
use nola_application::ApplicationCommand;
use nola_protocol::{ResolveUnknownEffectRecovery, UnknownEffectRecoveryChoice,
    UnknownEffectRecoveryInspection, UnknownEffectRecoveryReply};

fn text(values: [&'static str; 9]) -> &'static str {
    crate::i18n::pick_language(crate::i18n::current_language(), values)
}

pub(crate) fn usage() -> &'static str {
    text([
        "/recover 查看结果未知的操作；/recover retry <尝试ID> <理由> 授权风险重试；/recover block <理由> 保留未知并结束为受阻。重试可能重复产生副作用，授权不会立即执行。",
        "/recover 檢視結果未知的操作；/recover retry <嘗試ID> <理由> 授權風險重試；/recover block <理由> 保留未知並結束為受阻。重試可能重複產生副作用，授權不會立即執行。",
        "/recover inspects unknown effects; /recover retry <attempt-id> <reason> authorizes a risk-bearing retry; /recover block <reason> preserves unknown effects and closes as blocked. Retry may duplicate effects; authorization does not execute it.",
        "/recover prüft unbekannte Effekte; /recover retry <attempt-id> <reason> autorisiert einen risikobehafteten Wiederholungsversuch; /recover block <reason> behält unbekannte Effekte bei und schließt als blockiert ab. Ein Wiederholungsversuch kann Effekte duplizieren; die Autorisierung führt ihn nicht sofort aus.",
        "/recover inspecciona efectos desconocidos; /recover retry <attempt-id> <reason> autoriza un reintento con riesgo; /recover block <reason> conserva los efectos desconocidos y cierra como bloqueado. Reintentar puede duplicar efectos; la autorización no lo ejecuta de inmediato.",
        "/recover inspecte les effets inconnus ; /recover retry <attempt-id> <reason> autorise une nouvelle tentative risquée ; /recover block <reason> conserve les effets inconnus et clôture comme bloqué. Une nouvelle tentative peut dupliquer les effets ; l’autorisation ne l’exécute pas immédiatement.",
        "/recover で結果不明の操作を確認します。/recover retry <attempt-id> <reason> でリスク付き再試行を許可し、/recover block <reason> で不明な結果を保持したままブロック終了します。再試行は副作用を重複させる可能性があり、許可だけでは実行されません。",
        "/recover 로 결과가 불명확한 작업을 확인합니다. /recover retry <attempt-id> <reason> 은 위험 재시도를 승인하고, /recover block <reason> 은 불명확한 효과를 유지한 채 차단 상태로 종료합니다. 재시도는 부작용을 중복시킬 수 있으며 승인만으로 즉시 실행되지 않습니다.",
        "/recover проверяет операции с неизвестным результатом; /recover retry <attempt-id> <reason> разрешает рискованный повтор; /recover block <reason> сохраняет неизвестный статус и закрывает запуск как заблокированный. Повтор может продублировать побочные эффекты; разрешение не запускает его немедленно.",
    ])
}

pub(crate) fn execute(config: &Config, arguments: &[&str]) -> Result<String, String> {
    require_persistent_session(config.save_session)?;
    let conversation_id = config.session_id.as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            text([
                "请先选择会话。",
                "請先選擇工作階段。",
                "Select a conversation first.",
                "Wählen Sie zuerst eine Unterhaltung aus.",
                "Selecciona primero una conversación.",
                "Sélectionnez d’abord une conversation.",
                "先に会話を選択してください。",
                "먼저 대화를 선택하세요.",
                "Сначала выберите диалог.",
            ])
            .to_string()
        })?;
    let mut application = CliApplicationPort::persistent_default()?;
    let active = application.active_run(conversation_id)?
        .ok_or_else(|| {
            text([
                "此会话没有活动运行；已封存的结果不会被重开。",
                "此工作階段沒有活動執行；已封存的結果不會被重開。",
                "No active Run; sealed results are not reopened.",
                "Kein aktiver Lauf; abgeschlossene Ergebnisse werden nicht erneut geöffnet.",
                "No hay ninguna ejecución activa; los resultados cerrados no se reabren.",
                "Aucune exécution active ; les résultats scellés ne sont pas rouverts.",
                "アクティブな実行はありません。確定済みの結果は再開されません。",
                "활성 실행이 없습니다. 확정된 결과는 다시 열리지 않습니다.",
                "Нет активного запуска; закрытые результаты не открываются повторно.",
            ])
            .to_string()
        })?;
    let assistant_message_id = active.assistant_message_id
        .ok_or("active Run has no canonical assistant owner")?;
    let inspection: Option<UnknownEffectRecoveryInspection> = application.execute(
        ApplicationCommand::InspectUnknownEffectRecovery {
            conversation_id: conversation_id.into(), expected_run_id: active.run_id,
            assistant_message_id,
        },
    )?;
    let inspection = inspection.ok_or_else(|| {
        text([
            "运行已改变，请重新检查。",
            "執行已變更，請重新檢查。",
            "Run changed; inspect again.",
            "Der Lauf hat sich geändert; bitte erneut prüfen.",
            "La ejecución cambió; vuelve a inspeccionar.",
            "L’exécution a changé ; vérifiez à nouveau.",
            "実行状態が変更されました。再度確認してください。",
            "실행 상태가 변경되었습니다. 다시 확인하세요.",
            "Состояние запуска изменилось; проверьте снова.",
        ])
        .to_string()
    })?;
    if arguments.is_empty() {
        let mut lines = vec![format!("Run: {}", inspection.run_id), usage().into()];
        if inspection.targets.is_empty() {
            lines.push(
                text([
                    "没有结果未知的待处理操作。",
                    "沒有結果未知的待處理操作。",
                    "No pending effects with unknown outcomes.",
                    "Keine ausstehenden Effekte mit unbekanntem Ergebnis.",
                    "No hay efectos pendientes con resultado desconocido.",
                    "Aucun effet en attente au résultat inconnu.",
                    "結果が不明な保留中の操作はありません。",
                    "결과가 불명확한 대기 중인 작업이 없습니다.",
                    "Нет ожидающих операций с неизвестным результатом.",
                ])
                .into(),
            );
        }
        for target in &inspection.targets {
            // Never display or export fencing tokens or invocation digests.
            lines.push(format!("{} · {} · revision {}", target.attempt_id, target.effect_id, target.expected_attempt_revision));
        }
        if inspection.independent_pending_effects > 0 {
            lines.push(format!(
                "{}: {}",
                text([
                    "其他操作尚未收敛，暂不能受阻封存",
                    "其他操作尚未收斂，暫不能受阻封存",
                    "Other work must settle before blocking",
                    "Andere Vorgänge müssen zuerst abgeschlossen sein, bevor blockiert werden kann",
                    "Otras tareas deben finalizar antes de bloquear",
                    "D’autres opérations doivent se stabiliser avant le blocage",
                    "他の処理が収束するまでブロック終了できません",
                    "다른 작업이 정리될 때까지 차단 상태로 종료할 수 없습니다",
                    "Другие операции должны завершиться перед блокировкой",
                ]),
                inspection.independent_pending_effects
            ));
        }
        return Ok(lines.join("\n"));
    }
    let command = decision(inspection, arguments, uuid::Uuid::new_v4().to_string(),
        chrono::Utc::now().timestamp_millis())?;
    let reply: UnknownEffectRecoveryReply = application.execute(
        ApplicationCommand::ResolveUnknownEffectRecovery { input: command },
    )?;
    match reply {
        UnknownEffectRecoveryReply::RetryAuthorized { .. } => Ok(text([
            "风险重试已获授权，尚未执行；/resume 会打开会话选择器，请选择当前会话并检查其运行恢复状态。不要发送新消息代替恢复。原操作结果仍不被认定为未生效。",
            "風險重試已獲授權，尚未執行；/resume 會開啟工作階段選擇器，請選擇目前工作階段並檢查其執行恢復狀態。不要傳送新訊息代替恢復。原操作結果仍不被認定為未生效。",
            "Risk retry authorized, not executed. /resume opens the conversation picker; select this conversation and check its Run recovery status. Do not send a new message as a substitute for recovery. The original effect is not declared unapplied.",
            "Risiko-Wiederholung autorisiert, aber nicht ausgeführt. /resume öffnet die Unterhaltungsauswahl; wählen Sie diese Unterhaltung und prüfen Sie den Wiederherstellungsstatus. Senden Sie keine neue Nachricht als Ersatz. Der ursprüngliche Effekt gilt nicht als unausgeführt.",
            "Reintento con riesgo autorizado, pero no ejecutado. /resume abre el selector de conversaciones; selecciona esta conversación y revisa el estado de recuperación. No envíes un mensaje nuevo en lugar de recuperar. El efecto original no se declara sin aplicar.",
            "Nouvelle tentative risquée autorisée, mais non exécutée. /resume ouvre le sélecteur de conversations ; sélectionnez cette conversation et vérifiez l’état de reprise. N’envoyez pas un nouveau message à la place. L’effet initial n’est pas déclaré non appliqué.",
            "リスク付き再試行が許可されました（未実行）。/resume で会話一覧を開き、この会話を選択して実行復元状態を確認してください。復元の代わりに新しいメッセージを送らないでください。元の操作は未適用とは見なされません。",
            "위험 재시도가 승인되었지만 아직 실행되지 않았습니다. /resume 으로 대화 선택기를 열어 현재 대화를 선택하고 실행 복구 상태를 확인하세요. 복구 대신 새 메시지를 보내지 마세요. 원래 효과는 미적용으로 간주되지 않습니다.",
            "Рискованный повтор разрешён, но не выполнен. /resume открывает выбор диалогов; выберите этот диалог и проверьте статус восстановления. Не отправляйте новое сообщение вместо восстановления. Исходный эффект не считается невыполненным.",
        ]).into()),
        UnknownEffectRecoveryReply::UnknownPreservedBlocked { .. } => Ok(text([
            "已保留未知效果并封存为受阻；没有重试或标记成功。",
            "已保留未知效果並封存為受阻；沒有重試或標記成功。",
            "Unknown effects retained; Run sealed as blocked. Nothing was retried or marked successful.",
            "Unbekannte Effekte beibehalten; Lauf als blockiert abgeschlossen. Nichts wurde wiederholt oder als erfolgreich markiert.",
            "Efectos desconocidos conservados; ejecución cerrada como bloqueada. No se reintentó ni se marcó nada como exitoso.",
            "Effets inconnus conservés ; exécution clôturée comme bloquée. Rien n’a été relancé ni marqué comme réussi.",
            "不明な結果を保持したままブロック終了しました。再試行や成功扱いは行われていません。",
            "불명확한 효과를 유지한 채 차단 상태로 종료했습니다. 재시도하거나 성공으로 표시하지 않았습니다.",
            "Неизвестные эффекты сохранены; запуск закрыт как заблокированный. Ничего не повторялось и не помечалось успешным.",
        ]).into()),
        UnknownEffectRecoveryReply::AppliedReceiptVerified { .. } => Err("unexpected receipt-only reply to recovery decision".into()),
        UnknownEffectRecoveryReply::Rejected { code } | UnknownEffectRecoveryReply::Conflict { code } =>
            Err(format!(
                "{} ({code})",
                text([
                    "恢复决策未应用，请重新检查状态",
                    "恢復決策未套用，請重新檢查狀態",
                    "Recovery decision not applied; inspect again",
                    "Wiederherstellungsentscheidung nicht angewendet; bitte erneut prüfen",
                    "La decisión de recuperación no se aplicó; vuelve a inspeccionar",
                    "Décision de reprise non appliquée ; vérifiez à nouveau",
                    "復元判断は適用されませんでした。再度状態を確認してください",
                    "복구 결정이 적용되지 않았습니다. 상태를 다시 확인하세요",
                    "Решение по восстановлению не применено; проверьте состояние снова",
                ])
            )),
    }
}

fn require_persistent_session(save_session: bool) -> Result<(), String> {
    if !save_session {
        return Err(text([
            "无保存模式不支持 /recover；不会读取持久会话库。",
            "無儲存模式不支援 /recover；不會讀取持久工作階段庫。",
            "Recovery is unavailable in no-save mode; the persistent conversation store was not accessed.",
            "Wiederherstellung ist im Modus ohne Speichern nicht verfügbar; auf den dauerhaften Verlaufsspeicher wurde nicht zugegriffen.",
            "La recuperación no está disponible en modo sin guardado; no se accedió al almacenamiento persistente.",
            "La reprise n’est pas disponible en mode sans sauvegarde ; le stockage persistant n’a pas été consulté.",
            "保存なしモードでは /recover を利用できません。永続セッションストアにはアクセスしていません。",
            "저장 안 함 모드에서는 /recover 를 사용할 수 없습니다. 영구 대화 저장소에 접근하지 않았습니다.",
            "Восстановление недоступно в режиме без сохранения; постоянное хранилище диалогов не использовалось.",
        ]).into());
    }
    Ok(())
}

fn decision(
    inspection: UnknownEffectRecoveryInspection, arguments: &[&str], resolution_id: String, resolved_at_ms: i64,
) -> Result<ResolveUnknownEffectRecovery, String> {
    let (targets, choice) = match arguments {
        ["retry", attempt_id, reason @ ..] if !reason.is_empty() => {
            if !inspection.retry_allowed {
                return Err(text([
                    "取消尚未确认，只能保留未知结果。",
                    "取消尚未確認，只能保留未知結果。",
                    "Cancellation is unconfirmed; only retaining unknown is permitted.",
                    "Abbruch ist unbestätigt; nur das Beibehalten des unbekannten Zustands ist zulässig.",
                    "La cancelación no está confirmada; solo se permite conservar el estado desconocido.",
                    "L’annulation n’est pas confirmée ; seule la conservation de l’état inconnu est autorisée.",
                    "キャンセルが未確認のため、不明な結果の保持のみ許可されています。",
                    "취소가 확인되지 않아 불명확한 상태 유지에만 동의할 수 있습니다.",
                    "Отмена не подтверждена; допускается только сохранение неизвестного статуса.",
                ]).into());
            }
            let target = inspection.targets.iter().find(|target| target.attempt_id == *attempt_id)
                .ok_or_else(|| {
                    text([
                        "该尝试已改变或不属于当前运行，请重新检查。",
                        "該嘗試已變更或不屬於目前執行，請重新檢查。",
                        "Attempt changed or belongs to another Run; inspect again.",
                        "Dieser Versuch hat sich geändert oder gehört zu einem anderen Lauf; bitte erneut prüfen.",
                        "El intento cambió o pertenece a otra ejecución; vuelve a inspeccionar.",
                        "Cette tentative a changé ou appartient à une autre exécution ; vérifiez à nouveau.",
                        "その試行は変更されたか別の実行に属しています。再度確認してください。",
                        "해당 시도가 변경되었거나 다른 실행에 속합니다. 다시 확인하세요.",
                        "Эта попытка изменилась или относится к другому запуску; проверьте снова.",
                    ])
                    .to_string()
                })?;
            (vec![target.clone()], UnknownEffectRecoveryChoice::AuthorizeRiskRetry { reason: reason.join(" ") })
        }
        ["block", reason @ ..] if !reason.is_empty() => {
            if inspection.independent_pending_effects > 0 {
                return Err(text([
                    "其他操作尚未收敛，不能封存运行。",
                    "其他操作尚未收斂，不能封存執行。",
                    "Independent work is still pending; cannot seal the Run.",
                    "Unabhängige Vorgänge laufen noch; der Lauf kann nicht abgeschlossen werden.",
                    "Aún hay tareas independientes pendientes; no se puede cerrar la ejecución.",
                    "Des opérations indépendantes sont encore en cours ; impossible de clôturer l’exécution.",
                    "独立した処理がまだ保留中のため、実行を確定できません。",
                    "독립 작업이 아직 대기 중이므로 실행을 종료할 수 없습니다.",
                    "Независимые операции ещё не завершены; невозможно закрыть запуск.",
                ]).into());
            }
            (inspection.targets, UnknownEffectRecoveryChoice::KeepUnknownBlocked { reason: reason.join(" ") })
        }
        _ => return Err(usage().into()),
    };
    let command = ResolveUnknownEffectRecovery {
        conversation_id: inspection.conversation_id, run_id: inspection.run_id,
        invocation_config_digest: inspection.invocation_config_digest,
        resolution_id, targets, choice, resolved_at_ms,
    };
    command.validate()?;
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_save_recovery_stops_before_opening_the_persistent_port() {
        assert!(require_persistent_session(false).is_err());
        assert!(require_persistent_session(true).is_ok());
    }
    fn inspection() -> UnknownEffectRecoveryInspection {
        UnknownEffectRecoveryInspection {
            conversation_id: "conversation".into(), run_id: "run".into(), assistant_message_id: "assistant".into(),
            invocation_config_digest: "digest".into(), independent_pending_effects: 0, retry_allowed: true,
            targets: vec![nola_domain::unknown_effect_recovery::UnknownEffectRecoveryTarget {
                effect_id: "effect".into(), request_id: "request".into(), attempt_id: "attempt".into(),
                worker_generation: "generation".into(), lease_owner: "owner".into(), lease_token: "token".into(),
                expected_attempt_revision: 4,
            }],
        }
    }
    #[test]
    fn retry_uses_observed_fencing_and_never_accepts_applied_assertions() {
        let input = decision(inspection(), &["retry", "attempt", "accept duplicate risk"], "resolution".into(), 10).unwrap();
        assert_eq!(input.targets, inspection().targets);
        assert!(matches!(input.choice, UnknownEffectRecoveryChoice::AuthorizeRiskRetry { .. }));
        assert!(decision(inspection(), &["retry", "foreign", "reason"], "id".into(), 10).is_err());
        let mut cancelling = inspection(); cancelling.retry_allowed = false;
        assert!(decision(cancelling, &["retry", "attempt", "risk"], "id".into(), 10).is_err());
        assert!(decision(inspection(), &["applied", "attempt", "trust me"], "id".into(), 10).is_err());
    }
    #[test]
    fn blocked_requires_all_independent_work_to_settle() {
        let mut snapshot = inspection(); snapshot.independent_pending_effects = 1;
        assert!(decision(snapshot, &["block", "unknown"], "id".into(), 10).is_err());
        let input = decision(inspection(), &["block", "retain unknown"], "id".into(), 10).unwrap();
        assert!(matches!(input.choice, UnknownEffectRecoveryChoice::KeepUnknownBlocked { .. }));
    }
}
