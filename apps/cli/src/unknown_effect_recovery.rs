// SPDX-License-Identifier: Apache-2.0
//! Explicit user recovery controls. This module never creates or dispatches a
//! Run, and is deliberately absent from the model's tool catalog.
use crate::{application_port::CliApplicationPort, config::Config};
use nola_application::ApplicationCommand;
use nola_protocol::{ResolveUnknownEffectRecovery, UnknownEffectRecoveryChoice,
    UnknownEffectRecoveryInspection, UnknownEffectRecoveryReply};

fn text(zh: &'static str, en: &'static str) -> &'static str {
    if crate::i18n::current_language() == crate::i18n::Language::ZhHans { zh } else { en }
}

pub(crate) fn usage() -> &'static str {
    text("/recover 查看结果未知的操作；/recover retry <尝试ID> <理由> 授权风险重试；/recover block <理由> 保留未知并结束为受阻。重试可能重复产生副作用，授权不会立即执行。",
        "/recover inspects unknown effects; /recover retry <attempt-id> <reason> authorizes a risk-bearing retry; /recover block <reason> preserves unknown effects and closes as blocked. Retry may duplicate effects; authorization does not execute it.")
}

pub(crate) fn execute(config: &Config, arguments: &[&str]) -> Result<String, String> {
    require_persistent_session(config.save_session)?;
    let conversation_id = config.session_id.as_deref()
        .filter(|id| !id.trim().is_empty()).ok_or_else(|| text("请先选择会话。", "Select a conversation first.").to_string())?;
    let mut application = CliApplicationPort::persistent_default()?;
    let active = application.active_run(conversation_id)?
        .ok_or_else(|| text("此会话没有活动运行；已封存的结果不会被重开。", "No active Run; sealed results are not reopened.").to_string())?;
    let assistant_message_id = active.assistant_message_id
        .ok_or("active Run has no canonical assistant owner")?;
    let inspection: Option<UnknownEffectRecoveryInspection> = application.execute(
        ApplicationCommand::InspectUnknownEffectRecovery {
            conversation_id: conversation_id.into(), expected_run_id: active.run_id,
            assistant_message_id,
        },
    )?;
    let inspection = inspection.ok_or_else(|| text("运行已改变，请重新检查。", "Run changed; inspect again.").to_string())?;
    if arguments.is_empty() {
        let mut lines = vec![format!("Run: {}", inspection.run_id), usage().into()];
        if inspection.targets.is_empty() {
            lines.push(text("没有结果未知的待处理操作。", "No pending effects with unknown outcomes.").into());
        }
        for target in &inspection.targets {
            // Never display or export fencing tokens or invocation digests.
            lines.push(format!("{} · {} · revision {}", target.attempt_id, target.effect_id, target.expected_attempt_revision));
        }
        if inspection.independent_pending_effects > 0 {
            lines.push(format!("{}: {}", text("其他操作尚未收敛，暂不能受阻封存", "Other work must settle before blocking"), inspection.independent_pending_effects));
        }
        return Ok(lines.join("\n"));
    }
    let command = decision(inspection, arguments, uuid::Uuid::new_v4().to_string(),
        chrono::Utc::now().timestamp_millis())?;
    let reply: UnknownEffectRecoveryReply = application.execute(
        ApplicationCommand::ResolveUnknownEffectRecovery { input: command },
    )?;
    match reply {
        UnknownEffectRecoveryReply::RetryAuthorized { .. } => Ok(text(
            "风险重试已获授权，尚未执行；/resume 会打开会话选择器，请选择当前会话并检查其运行恢复状态。不要发送新消息代替恢复。原操作结果仍不被认定为未生效。",
            "Risk retry authorized, not executed. /resume opens the conversation picker; select this conversation and check its Run recovery status. Do not send a new message as a substitute for recovery. The original effect is not declared unapplied.").into()),
        UnknownEffectRecoveryReply::UnknownPreservedBlocked { .. } => Ok(text(
            "已保留未知效果并封存为受阻；没有重试或标记成功。",
            "Unknown effects retained; Run sealed as blocked. Nothing was retried or marked successful.").into()),
        UnknownEffectRecoveryReply::AppliedReceiptVerified { .. } => Err("unexpected receipt-only reply to recovery decision".into()),
        UnknownEffectRecoveryReply::Rejected { code } | UnknownEffectRecoveryReply::Conflict { code } =>
            Err(format!("{} ({code})", text("恢复决策未应用，请重新检查状态", "Recovery decision not applied; inspect again"))),
    }
}

fn require_persistent_session(save_session: bool) -> Result<(), String> {
    if !save_session {
        return Err(text("无保存模式不支持 /recover；不会读取持久会话库。", "Recovery is unavailable in no-save mode; the persistent conversation store was not accessed.").into());
    }
    Ok(())
}

fn decision(
    inspection: UnknownEffectRecoveryInspection, arguments: &[&str], resolution_id: String, resolved_at_ms: i64,
) -> Result<ResolveUnknownEffectRecovery, String> {
    let (targets, choice) = match arguments {
        ["retry", attempt_id, reason @ ..] if !reason.is_empty() => {
            if !inspection.retry_allowed {
                return Err(text("取消尚未确认，只能保留未知结果。", "Cancellation is unconfirmed; only retaining unknown is permitted.").into());
            }
            let target = inspection.targets.iter().find(|target| target.attempt_id == *attempt_id)
                .ok_or_else(|| text("该尝试已改变或不属于当前运行，请重新检查。", "Attempt changed or belongs to another Run; inspect again.").to_string())?;
            (vec![target.clone()], UnknownEffectRecoveryChoice::AuthorizeRiskRetry { reason: reason.join(" ") })
        }
        ["block", reason @ ..] if !reason.is_empty() => {
            if inspection.independent_pending_effects > 0 {
                return Err(text("其他操作尚未收敛，不能封存运行。", "Independent work is still pending; cannot seal the Run.").into());
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
