// SPDX-License-Identifier: Apache-2.0
use crate::{NolaStore, StoreError};
use nola_store_port::store_maintenance::*;
impl StoreMaintenanceRepository for NolaStore {
    type Error = StoreError;
    fn with_run_recovery_plan<F>(
        &mut self,
        conversation_id: Option<&str>,
        reason: &str,
        interrupted_at: &str,
        interrupted_at_ms: i64,
        planner: F,
    ) -> Result<usize, Self::Error>
    where
        F: FnMut(&RunRecoveryRecord) -> Result<RunRecoveryPlan, String>,
    {
        crate::run_recovery::with_plan(
            self,
            conversation_id,
            reason,
            interrupted_at,
            interrupted_at_ms,
            planner,
        )
    }
}
