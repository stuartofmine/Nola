// SPDX-License-Identifier: Apache-2.0
//! Host-owned memory authority and per-Run admission. Client intent can only
//! restrict authority; restored policy never substitutes for current authority.
use crate::host_invocation_request::HostInvocationRequest;

fn requested_long_term_memory_enabled() -> Option<bool> {
    std::env::var("NOLA_LONG_TERM_MEMORY_ENABLED")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
}

fn memory_policy_with_requested_state(
    mut policy: nola_core::memory::MemoryPolicy,
    enabled: Option<bool>,
) -> nola_core::memory::MemoryPolicy {
    if let Some(enabled) = enabled {
        policy.long_term_enabled = enabled;
        policy.automatic_extraction_enabled = enabled;
    }
    policy
}

pub(crate) fn constrain_memory_policy(
    mut host: nola_core::memory::MemoryPolicy,
    requested: Option<bool>,
) -> Result<nola_core::memory::MemoryPolicy, String> {
    if requested == Some(true) && !host.long_term_enabled {
        return Err("long-term memory is disabled by the Host".into());
    }
    if requested == Some(false) {
        host.long_term_enabled = false;
        host.automatic_extraction_enabled = false;
    }
    Ok(host)
}

/// Resolve Host authority independently of the preceding Turn's client restriction.
pub(crate) fn resolve_host_memory_policy(
    requested: Option<bool>,
) -> Result<nola_core::memory::MemoryPolicy, String> {
    let host = memory_policy_with_requested_state(
        nola_core::memory::MemoryPolicy::default(),
        requested_long_term_memory_enabled(),
    );
    constrain_memory_policy(host, requested)
}

pub(crate) fn validate_memory_policy_ceiling(
    frozen: &nola_core::memory::MemoryPolicy,
    current: &nola_core::memory::MemoryPolicy,
) -> Result<(), String> {
    if (frozen.long_term_enabled && !current.long_term_enabled)
        || (frozen.automatic_extraction_enabled && !current.automatic_extraction_enabled)
    {
        return Err("Host memory authority was revoked; the frozen Run cannot execute".into());
    }
    Ok(())
}

pub(crate) fn restore_host_memory_admission(
    config: &mut HostInvocationRequest,
    frozen: &nola_core::memory::MemoryPolicy,
) -> Result<(), String> {
    validate_memory_policy_ceiling(frozen, &resolve_host_memory_policy(None)?)?;
    config.runtime.admitted_memory_policy = Some(frozen.clone());
    Ok(())
}

pub(crate) fn freeze_host_memory_admission(config: &mut HostInvocationRequest) -> Result<(), String> {
    if let Some(frozen) = config.runtime.admitted_memory_policy.as_ref() {
        validate_memory_policy_ceiling(frozen, &resolve_host_memory_policy(None)?)?;
    } else {
        config.runtime.admitted_memory_policy =
            Some(resolve_host_memory_policy(config.runtime.memory_enabled)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{constrain_memory_policy, memory_policy_with_requested_state};
    use nola_core::memory::MemoryPolicy;
    #[test]
    fn requested_memory_state_updates_both_recall_and_extraction_policy() {
        let base = MemoryPolicy {
            search_limit: 7,
            minimum_confidence: 0.8,
            ..MemoryPolicy::default()
        };
        let disabled = memory_policy_with_requested_state(base.clone(), Some(false));
        assert!(!disabled.long_term_enabled);
        assert!(!disabled.automatic_extraction_enabled);
        assert_eq!(disabled.search_limit, 7);
        assert_eq!(disabled.minimum_confidence, 0.8);

        assert_eq!(memory_policy_with_requested_state(base.clone(), None), base);
    }

    #[test]
    fn client_memory_intent_only_restricts_independent_host_policy() {
        let host = MemoryPolicy::default();
        assert!(host.long_term_enabled);
        assert!(!host.automatic_extraction_enabled);
        for requested in [None, Some(true)] {
            assert_eq!(
                constrain_memory_policy(host.clone(), requested).unwrap(),
                host
            );
        }
        let disabled = constrain_memory_policy(host.clone(), Some(false)).unwrap();
        assert!(!disabled.long_term_enabled);
        assert!(!disabled.automatic_extraction_enabled);
        // A subsequent Turn resolves against Host authority, not this client's old policy.
        assert_eq!(
            constrain_memory_policy(host.clone(), Some(true)).unwrap(),
            host
        );
        assert!(constrain_memory_policy(disabled.clone(), Some(true)).is_err());
        assert_eq!(
            constrain_memory_policy(disabled.clone(), None).unwrap(),
            disabled
        );
        let extraction_enabled = MemoryPolicy {
            automatic_extraction_enabled: true,
            ..host
        };
        let restricted = constrain_memory_policy(extraction_enabled.clone(), Some(false)).unwrap();
        assert!(!restricted.long_term_enabled && !restricted.automatic_extraction_enabled);
        assert!(
            extraction_enabled.long_term_enabled && extraction_enabled.automatic_extraction_enabled
        );
    }
}
