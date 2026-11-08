// SPDX-License-Identifier: Apache-2.0
//! Shared typed resource budgets and measured usage; no execution authority.
use serde::{Deserialize, Serialize};

pub const SCHEDULING_METADATA_KEY: &str = "_nola_scheduler";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskBudgetFailure {
    pub resource: RuntimeTaskBudgetResource,
    pub limit: u64,
    pub observed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTaskBudgetResource {
    WallTimeMs,
    CpuTimeMs,
    MemoryBytes,
    Processes,
    OutputBytes,
}

impl RuntimeTaskBudgetResource {
    pub fn code(self) -> &'static str {
        match self {
            Self::WallTimeMs => "wall_time_ms",
            Self::CpuTimeMs => "cpu_time_ms",
            Self::MemoryBytes => "memory_bytes",
            Self::Processes => "processes",
            Self::OutputBytes => "output_bytes",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTaskPriority {
    Low,
    #[default]
    Normal,
    High,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskResourceBudget {
    pub max_wall_time_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub max_cpu_time_ms: Option<u64>,
    pub max_memory_bytes: Option<u64>,
    pub max_processes: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskResourceUsage {
    pub cpu_time_ms: u64,
    pub peak_memory_bytes: u64,
    pub peak_processes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskScheduling {
    pub priority: RuntimeTaskPriority,
    pub queued_at_ms: i64,
    pub resource_budget: RuntimeTaskResourceBudget,
    #[serde(default)]
    pub output_bytes: u64,
    #[serde(default)]
    pub resource_usage: RuntimeTaskResourceUsage,
    #[serde(default)]
    pub budget_exhaustion: Option<String>,
}

impl RuntimeTaskScheduling {
    /// Evaluate execution admission from current durable observations. This
    /// does not grant execution authority or repair missing usage evidence.
    pub fn execution_budget_failure(
        &self,
        started_at_ms: Option<i64>,
        now_ms: i64,
    ) -> Result<Option<RuntimeTaskBudgetFailure>, String> {
        self.validate()?;
        let elapsed = if self.resource_budget.max_wall_time_ms.is_some() {
            let start = started_at_ms.filter(|start| *start >= 0 && *start <= now_ms)
                .ok_or("execution wall-clock budget has no valid task start")?;
            Some((now_ms - start) as u64)
        } else {
            None
        };
        // A persisted exhaustion cannot be erased or trusted without evidence.
        if self.budget_exhaustion.is_some() {
            return self.measured_budget_failure(started_at_ms, now_ms);
        }
        use RuntimeTaskBudgetResource::*;
        for (resource, limit, observed) in [
            (WallTimeMs, self.resource_budget.max_wall_time_ms, elapsed.unwrap_or(0)),
            (OutputBytes, self.resource_budget.max_output_bytes, self.output_bytes),
            (CpuTimeMs, self.resource_budget.max_cpu_time_ms, self.resource_usage.cpu_time_ms),
            (MemoryBytes, self.resource_budget.max_memory_bytes, self.resource_usage.peak_memory_bytes),
            (Processes, self.resource_budget.max_processes.map(u64::from), u64::from(self.resource_usage.peak_processes)),
        ] {
            if let Some(limit) = limit {
                if observed > limit || (resource == WallTimeMs && observed == limit) {
                    return Ok(Some(RuntimeTaskBudgetFailure { resource, limit, observed }));
                }
            }
        }
        Ok(None)
    }

    pub fn measured_budget_failure(
        &self,
        started_at_ms: Option<i64>,
        now_ms: i64,
    ) -> Result<Option<RuntimeTaskBudgetFailure>, String> {
        self.validate()?;
        let Some(code) = self.budget_exhaustion.as_deref() else {
            return Ok(None);
        };
        use RuntimeTaskBudgetResource::*;
        let (resource, limit, observed) = match code {
            "cpu_time_ms" => (
                CpuTimeMs,
                self.resource_budget.max_cpu_time_ms,
                self.resource_usage.cpu_time_ms,
            ),
            "memory_bytes" => (
                MemoryBytes,
                self.resource_budget.max_memory_bytes,
                self.resource_usage.peak_memory_bytes,
            ),
            "processes" => (
                Processes,
                self.resource_budget.max_processes.map(u64::from),
                u64::from(self.resource_usage.peak_processes),
            ),
            "output_bytes" => (
                OutputBytes,
                self.resource_budget.max_output_bytes,
                self.output_bytes,
            ),
            "wall_time_ms" => {
                let start = started_at_ms
                    .filter(|start| *start >= 0 && *start <= now_ms)
                    .ok_or("wall-clock exhaustion has no valid task start")?;
                (
                    WallTimeMs,
                    self.resource_budget.max_wall_time_ms,
                    (now_ms - start) as u64,
                )
            }
            _ => return Err("unknown task budget exhaustion resource".into()),
        };
        let limit = limit.ok_or("budget exhaustion has no configured resource limit")?;
        if observed < limit || (observed == limit && resource != WallTimeMs) {
            return Err("budget exhaustion is not supported by measured usage".into());
        }
        Ok(Some(RuntimeTaskBudgetFailure {
            resource,
            limit,
            observed,
        }))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.queued_at_ms < 0 {
            return Err("runtime task queued_at_ms must be non-negative".into());
        }
        if self.resource_budget.max_wall_time_ms == Some(0) {
            return Err("runtime task max_wall_time_ms must be positive".into());
        }
        if self.resource_budget.max_output_bytes == Some(0) {
            return Err("runtime task max_output_bytes must be positive".into());
        }
        if self.resource_budget.max_cpu_time_ms == Some(0) {
            return Err("runtime task max_cpu_time_ms must be positive".into());
        }
        if self.resource_budget.max_memory_bytes == Some(0) {
            return Err("runtime task max_memory_bytes must be positive".into());
        }
        if self.resource_budget.max_processes == Some(0) {
            return Err("runtime task max_processes must be positive".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_admission_uses_original_wall_start_and_validates_sticky_evidence() {
        let mut scheduling = RuntimeTaskScheduling {
            priority: RuntimeTaskPriority::Normal,
            queued_at_ms: 1,
            resource_budget: RuntimeTaskResourceBudget {
                max_wall_time_ms: Some(100),
                max_cpu_time_ms: Some(10),
                ..Default::default()
            },
            output_bytes: 0,
            resource_usage: Default::default(),
            budget_exhaustion: None,
        };
        assert!(scheduling.execution_budget_failure(None, 100).is_err());
        assert!(scheduling.execution_budget_failure(Some(101), 100).is_err());
        assert!(scheduling.execution_budget_failure(Some(-1), 100).is_err());
        assert_eq!(scheduling.execution_budget_failure(Some(1), 100).unwrap(), None);
        assert_eq!(scheduling.execution_budget_failure(Some(1), 101).unwrap().unwrap(),
            RuntimeTaskBudgetFailure { resource: RuntimeTaskBudgetResource::WallTimeMs, limit: 100, observed: 100 });
        // Admission detects exhaustion without manufacturing a persisted code.
        assert!(scheduling.budget_exhaustion.is_none());
        assert_eq!(scheduling.measured_budget_failure(Some(1), 101).unwrap(), None);
        scheduling.budget_exhaustion = Some("cpu_time_ms".into());
        assert!(scheduling.execution_budget_failure(Some(1), 101).is_err(),
            "an invalid sticky code must not be replaced by another true limit");
        scheduling.resource_usage.cpu_time_ms = 11;
        assert_eq!(scheduling.execution_budget_failure(Some(1), 101).unwrap().unwrap().resource,
            RuntimeTaskBudgetResource::CpuTimeMs);
    }

    #[test]
    fn non_wall_limits_are_strictly_exceeded_not_merely_reached() {
        for resource in [RuntimeTaskBudgetResource::CpuTimeMs, RuntimeTaskBudgetResource::MemoryBytes,
            RuntimeTaskBudgetResource::Processes, RuntimeTaskBudgetResource::OutputBytes] {
            let mut scheduling = RuntimeTaskScheduling {
                priority: RuntimeTaskPriority::Normal, queued_at_ms: 1,
                resource_budget: Default::default(), output_bytes: 10,
                resource_usage: RuntimeTaskResourceUsage {
                    cpu_time_ms: 10, peak_memory_bytes: 10, peak_processes: 10,
                }, budget_exhaustion: None,
            };
            let set_limit = |s: &mut RuntimeTaskScheduling, limit| match resource {
                RuntimeTaskBudgetResource::CpuTimeMs => s.resource_budget.max_cpu_time_ms = Some(limit),
                RuntimeTaskBudgetResource::MemoryBytes => s.resource_budget.max_memory_bytes = Some(limit),
                RuntimeTaskBudgetResource::Processes => s.resource_budget.max_processes = Some(limit as u32),
                RuntimeTaskBudgetResource::OutputBytes => s.resource_budget.max_output_bytes = Some(limit),
                RuntimeTaskBudgetResource::WallTimeMs => unreachable!(),
            };
            set_limit(&mut scheduling, 10);
            assert_eq!(scheduling.execution_budget_failure(None, 1).unwrap(), None);
            set_limit(&mut scheduling, 9);
            let failure = scheduling.execution_budget_failure(None, 1).unwrap().unwrap();
            assert_eq!(failure.resource, resource);
            scheduling.budget_exhaustion = Some(resource.code().into());
            assert_eq!(scheduling.measured_budget_failure(None, 1).unwrap(), Some(failure));
        }
    }

    #[test]
    fn exhausted_budget_requires_typed_code_limit_and_measured_usage() {
        let mut scheduling = RuntimeTaskScheduling {
            priority: RuntimeTaskPriority::Normal,
            queued_at_ms: 1,
            resource_budget: RuntimeTaskResourceBudget {
                max_cpu_time_ms: Some(10),
                ..Default::default()
            },
            output_bytes: 0,
            resource_usage: RuntimeTaskResourceUsage::default(),
            budget_exhaustion: None,
        };
        assert_eq!(
            scheduling.measured_budget_failure(Some(1), 100).unwrap(),
            None
        );
        scheduling.budget_exhaustion = Some("cpu_time_ms".into());
        assert!(scheduling.measured_budget_failure(Some(1), 100).is_err());
        scheduling.resource_usage.cpu_time_ms = 11;
        assert_eq!(
            scheduling.measured_budget_failure(Some(1), 100).unwrap(),
            Some(RuntimeTaskBudgetFailure {
                resource: RuntimeTaskBudgetResource::CpuTimeMs,
                limit: 10,
                observed: 11
            })
        );
        scheduling.budget_exhaustion = Some("unknown".into());
        assert!(scheduling.measured_budget_failure(Some(1), 100).is_err());
        scheduling.budget_exhaustion = Some("wall_time_ms".into());
        scheduling.resource_budget.max_wall_time_ms = Some(99);
        assert!(scheduling.measured_budget_failure(None, 100).is_err());
        assert_eq!(
            scheduling
                .measured_budget_failure(Some(1), 100)
                .unwrap()
                .unwrap()
                .observed,
            99
        );
    }
}
