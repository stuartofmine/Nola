// SPDX-License-Identifier: Apache-2.0
//! Shared worker identity and trusted Host OS-observation semantics. No OS I/O.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostWorkerProcessIdentity {
    pub pid: u32,
    pub boot_id: String,
    pub start_id: String,
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostWorkerLeaseIdentity {
    pub worker_generation: String,
    pub lease_owner: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostProcessObservation {
    Running {
        pid: u32,
        boot_id: String,
        start_id: String,
    },
    Gone {
        pid: u32,
        boot_id: String,
    },
    Unknown {
        pid: u32,
    },
}

fn field(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
}
impl HostWorkerProcessIdentity {
    pub fn validate(&self) -> Result<(), String> {
        if self.pid == 0
            || self.pid > i32::MAX as u32
            || !field(&self.boot_id)
            || !field(&self.start_id)
            || !field(&self.nonce)
        {
            return Err("invalid Host worker process identity".into());
        }
        Ok(())
    }
    pub fn lease_identity(&self) -> Result<HostWorkerLeaseIdentity, String> {
        self.validate()?;
        let identity = format!(
            "nola-process|{}|{}|{}|{}",
            self.pid, self.boot_id, self.start_id, self.nonce
        );
        Ok(HostWorkerLeaseIdentity {
            worker_generation: format!("generation|{identity}"),
            lease_owner: format!("owner|{identity}"),
        })
    }
    pub fn from_lease(worker_generation: &str, lease_owner: &str) -> Result<Self, String> {
        let generation = worker_generation
            .strip_prefix("generation|")
            .ok_or("worker has no process identity")?;
        let owner = lease_owner
            .strip_prefix("owner|")
            .ok_or("owner has no process identity")?;
        if generation != owner {
            return Err("worker generation and process owner differ".into());
        }
        let fields: Vec<_> = owner.split('|').collect();
        if fields.len() != 5 || fields[0] != "nola-process" {
            return Err("invalid worker process identity encoding".into());
        }
        let value = Self {
            pid: fields[1].parse().map_err(|_| "invalid worker PID")?,
            boot_id: fields[2].into(),
            start_id: fields[3].into(),
            nonce: fields[4].into(),
        };
        value.validate()?;
        if value.lease_identity()?.lease_owner != lease_owner {
            return Err("noncanonical worker process identity".into());
        }
        Ok(value)
    }
    pub fn is_terminated(&self, observed: &HostProcessObservation) -> Result<bool, String> {
        self.validate()?;
        let (pid, boot, start) = match observed {
            HostProcessObservation::Unknown { .. } => {
                return Err("Host process observation is unavailable".into())
            }
            HostProcessObservation::Gone { pid, boot_id } => (*pid, boot_id, None),
            HostProcessObservation::Running {
                pid,
                boot_id,
                start_id,
            } => (*pid, boot_id, Some(start_id)),
        };
        if pid != self.pid || !field(boot) || start.is_some_and(|s| !field(s)) {
            return Err("Host process observation identity mismatch".into());
        }
        Ok(boot != &self.boot_id || start.is_none() || start.is_some_and(|s| s != &self.start_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner() -> HostWorkerProcessIdentity {
        HostWorkerProcessIdentity {
            pid: 42,
            boot_id: "boot".into(),
            start_id: "123".into(),
            nonce: "unique".into(),
        }
    }
    #[test]
    fn exact_encoding_rejects_unwitnessed_and_mismatched_workers() {
        let owner = owner();
        let lease = owner.lease_identity().unwrap();
        assert_eq!(
            HostWorkerProcessIdentity::from_lease(&lease.worker_generation, &lease.lease_owner)
                .unwrap(),
            owner
        );
        assert!(HostWorkerProcessIdentity::from_lease("uuid", "android-device-uuid").is_err());
        assert!(HostWorkerProcessIdentity::from_lease(
            &lease.worker_generation,
            &lease.lease_owner.replace("unique", "other")
        )
        .is_err());
    }
    #[test]
    fn pid_reuse_boot_change_and_missing_process_are_distinct_from_unknown() {
        let o = owner();
        assert!(!o
            .is_terminated(&HostProcessObservation::Running {
                pid: 42,
                boot_id: "boot".into(),
                start_id: "123".into()
            })
            .unwrap());
        assert!(o
            .is_terminated(&HostProcessObservation::Running {
                pid: 42,
                boot_id: "boot".into(),
                start_id: "124".into()
            })
            .unwrap());
        assert!(o
            .is_terminated(&HostProcessObservation::Gone {
                pid: 42,
                boot_id: "boot".into()
            })
            .unwrap());
        assert!(o
            .is_terminated(&HostProcessObservation::Running {
                pid: 42,
                boot_id: "new-boot".into(),
                start_id: "123".into()
            })
            .unwrap());
        assert!(o
            .is_terminated(&HostProcessObservation::Unknown { pid: 42 })
            .is_err());
        assert!(o
            .is_terminated(&HostProcessObservation::Gone {
                pid: 43,
                boot_id: "boot".into()
            })
            .is_err());
    }
}

/// An unavailable OS identity still has a fenced current worker namespace.
/// It supplies no evidence that this worker has stopped after a process loss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostWorkerIdentity {
    Process(HostWorkerProcessIdentity),
    Unobservable { nonce: String },
}
impl HostWorkerIdentity {
    pub fn lease_identity(&self) -> Result<HostWorkerLeaseIdentity, String> {
        match self {
            Self::Process(process) => process.lease_identity(),
            Self::Unobservable { nonce } => {
                if !field(nonce) {
                    return Err("invalid unobservable worker nonce".into());
                }
                let identity = format!("nola-unobservable|{nonce}");
                Ok(HostWorkerLeaseIdentity {
                    worker_generation: format!("generation|{identity}"),
                    lease_owner: format!("owner|{identity}"),
                })
            }
        }
    }
    pub fn from_lease(generation: &str, owner: &str) -> Result<Self, String> {
        let encoded = generation
            .strip_prefix("generation|")
            .ok_or("missing typed worker identity")?;
        if owner.strip_prefix("owner|") != Some(encoded) {
            return Err("worker generation and owner differ".into());
        }
        if let Some(nonce) = encoded.strip_prefix("nola-unobservable|") {
            let value = Self::Unobservable {
                nonce: nonce.into(),
            };
            if value.lease_identity()?.lease_owner != owner {
                return Err("noncanonical worker identity".into());
            }
            Ok(value)
        } else {
            HostWorkerProcessIdentity::from_lease(generation, owner).map(Self::Process)
        }
    }
}

#[cfg(test)]
mod unobservable_tests {
    use super::*;
    #[test]
    fn unavailable_observation_preserves_fencing_without_proving_death() {
        let identity: HostWorkerIdentity = serde_json::from_value(
            serde_json::json!({"kind":"unobservable","nonce":"current-worker"}),
        )
        .unwrap();
        let lease = identity.lease_identity().unwrap();
        assert_eq!(
            HostWorkerIdentity::from_lease(&lease.worker_generation, &lease.lease_owner).unwrap(),
            identity
        );
        assert!(HostWorkerProcessIdentity::from_lease(
            &lease.worker_generation,
            &lease.lease_owner
        )
        .is_err());
        assert!(serde_json::from_value::<HostWorkerIdentity>(
            serde_json::json!({"kind":"unobservable","nonce":"current-worker","pid":42})
        )
        .is_err());
        let process:HostWorkerIdentity=serde_json::from_value(serde_json::json!({"kind":"process","pid":42,"boot_id":"boot","start_id":"100","nonce":"current-worker"})).unwrap();
        assert!(process.lease_identity().is_ok());
    }
}
