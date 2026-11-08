//! Durable Host effect dispatch built on the canonical outbox.
//!
//! The application transaction creates effects. Hosts only dispatch committed
//! rows and acknowledge them after the external side effect succeeds. A crash
//! before acknowledgement deliberately leaves the row pending for replay.

use crate::NolaStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchEffect {
    pub effect_id: String,
    pub request_id: String,
    pub effect_kind: String,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReplayReport {
    pub dispatched: usize,
    pub acknowledged: usize,
    pub stopped_after_failure: bool,
}

/// Dispatches committed effects in store order.
///
/// Dispatch is intentionally serial: later effects may depend on earlier Host
/// work. The first failure stops the pass and remains pending. Callers can run
/// another pass after backoff or process restart.
pub fn replay_pending_effects<F>(
    store: &mut NolaStore,
    run_id: &str,
    limit: usize,
    acknowledged_at_ms: i64,
    mut dispatch: F,
) -> Result<ReplayReport, String>
where
    F: FnMut(&DispatchEffect) -> Result<(), String>,
{
    let pending = store
        .pending_effects(run_id)
        .map_err(|error| error.to_string())?;
    let mut report = ReplayReport::default();

    for stored in pending.into_iter().take(limit) {
        let effect = DispatchEffect {
            effect_id: stored.effect_id.clone(),
            request_id: stored.request_id.clone(),
            effect_kind: stored.effect_kind.clone(),
            payload_json: stored.payload_json.clone(),
        };
        report.dispatched += 1;
        if dispatch(&effect).is_err() {
            report.stopped_after_failure = true;
            break;
        }
        store
            .acknowledge_effect(&effect.effect_id, acknowledged_at_ms)
            .map_err(|error| error.to_string())?;
        report.acknowledged += 1;
    }

    Ok(report)
}
