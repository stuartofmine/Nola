// SPDX-License-Identifier: Apache-2.0
//! One current Shell-worker dispatch boundary; zero PID is not non-execution.
use crate::assistant_journal::AssistantDigest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SHELL_WORKER_DISPATCH_METADATA_KEY: &str = "_nola_shell_worker_dispatch";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
pub enum ShellWorkerDispatchPhase { Prepared, MayHaveExecuted }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellWorkerDispatchWitness {
    pub task_id: String,
    pub attempt_id: String,
    pub worker_id: String,
    pub worker_generation: String,
    pub lease_token_digest: AssistantDigest,
    pub phase: ShellWorkerDispatchPhase,
}

impl ShellWorkerDispatchWitness {
    pub fn new(task_id: &str, attempt_id: &str, worker_id: &str, generation: &str, token: &str,
        phase: ShellWorkerDispatchPhase) -> Result<Self,String> {
        let witness = Self { task_id:task_id.into(), attempt_id:attempt_id.into(), worker_id:worker_id.into(),
            worker_generation:generation.into(), lease_token_digest:token_digest(token), phase };
        witness.validate()?;
        if token.trim().is_empty() { return Err("Shell dispatch token is empty".into()); }
        Ok(witness)
    }

    pub fn validate(&self) -> Result<(),String> {
        if [&self.task_id,&self.attempt_id,&self.worker_id,&self.worker_generation].iter()
            .any(|value| value.trim().is_empty() || value.len()>512 || value.chars().any(char::is_control)) {
            return Err("invalid Shell dispatch identity".into());
        }
        Ok(())
    }

    pub fn matches(&self, task: &str, attempt: &str, worker: &str, generation: &str, token: &str) -> bool {
        self.validate().is_ok() && self.task_id==task && self.attempt_id==attempt && self.worker_id==worker
            && self.worker_generation==generation && !token.is_empty() && self.lease_token_digest==token_digest(token)
    }

    pub fn follows(&self, previous: Option<&Self>) -> Result<(),String> {
        self.validate()?;
        if let Some(previous) = previous {
            previous.validate()?;
            if previous.task_id!=self.task_id { return Err("Shell dispatch Task owner changed".into()); }
            if previous.attempt_id==self.attempt_id {
                let mut expected = previous.clone(); expected.phase=self.phase;
                if expected!=*self || (previous.phase==ShellWorkerDispatchPhase::MayHaveExecuted
                    && self.phase==ShellWorkerDispatchPhase::Prepared) {
                    return Err("Shell dispatch identity changed or admission moved backwards".into());
                }
                return Ok(());
            }
        }
        if self.phase!=ShellWorkerDispatchPhase::Prepared {
            return Err("Shell worker dispatch requires its original Prepared witness".into());
        }
        Ok(())
    }
}

fn token_digest(token: &str) -> AssistantDigest {
    AssistantDigest::parse_hex(&format!("{:x}",Sha256::digest(token.as_bytes()))).expect("SHA-256 hex")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dispatch_is_bound_and_never_returns_to_prepared() {
        let prepared=ShellWorkerDispatchWitness::new("task","attempt","worker","generation","token",ShellWorkerDispatchPhase::Prepared).unwrap();
        prepared.follows(None).unwrap();
        assert!(prepared.matches("task","attempt","worker","generation","token"));
        for (attempt,token) in [("other","token"),("attempt","foreign")] {
            assert!(!prepared.matches("task",attempt,"worker","generation",token));
        }
        let mut dispatched=prepared.clone(); dispatched.phase=ShellWorkerDispatchPhase::MayHaveExecuted;
        assert!(dispatched.follows(None).is_err());
        dispatched.follows(Some(&prepared)).unwrap();
        assert!(prepared.follows(Some(&dispatched)).is_err());
        let mut foreign=dispatched.clone(); foreign.worker_generation="other".into();
        assert!(foreign.follows(Some(&dispatched)).is_err());
        let encoded=serde_json::to_value(prepared).unwrap();
        let mut missing=encoded.clone(); missing.as_object_mut().unwrap().remove("phase");
        assert!(serde_json::from_value::<ShellWorkerDispatchWitness>(missing).is_err());
        let mut extra=encoded; extra["pid"]=serde_json::json!(0);
        assert!(serde_json::from_value::<ShellWorkerDispatchWitness>(extra).is_err());
    }
}
