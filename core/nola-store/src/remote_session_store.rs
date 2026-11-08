// SPDX-License-Identifier: Apache-2.0
//! Exact immutable local invocation reads; remote replicas never own execution configuration.

use super::*;

impl NolaStore {
    pub fn load_run_invocation_config(
        &self,
        run_id: &str,
    ) -> StoreResult<Option<StoredRunInvocationConfig>> {
        require_id("run_id", run_id)?;
        let record = self
            .connection
            .query_row(
                "SELECT r.id, p.config_json, r.invocation_config_digest,
                    EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id)
                 FROM runs r LEFT JOIN run_invocation_payloads p ON p.run_id=r.id
                 WHERE r.id = ?1",
                params![run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((run_id, config, digest, remote)) = record else {
            return Ok(None);
        };
        if remote {
            if config.is_some() || digest.is_some() {
                return Err(StoreError::Invariant(
                    "remote Run cannot own local invocation configuration".into(),
                ));
            }
            return Ok(None);
        }
        match (config, digest) {
            (Some(invocation_config_json), Some(invocation_config_digest)) => {
                Ok(Some(StoredRunInvocationConfig {
                    run_id,
                    invocation_config_json,
                    invocation_config_digest,
                }))
            }
            (None, None) => Ok(None), // Historical metadata, not an executable source Run.
            _ => Err(StoreError::Invariant(
                "local Run lost its frozen invocation payload or digest".into(),
            )),
        }
    }
}
