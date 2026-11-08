// SPDX-License-Identifier: Apache-2.0
//! Host-owned invocation binding for local process lifecycle observations.

use crate::execution_context::ScopeStack;
use nola_domain::execution_evidence::LocalExecutionIdentity;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

thread_local! {
    static LOCAL_EXECUTION: RefCell<ScopeStack<LocalExecutionIdentity>> =
        const { RefCell::new(ScopeStack::new()) };
}

pub(crate) struct LocalExecutionIdentityGuard {
    key: u64,
    _same_thread: PhantomData<Rc<()>>,
}

pub(crate) fn enter_local_execution_identity(
    identity: LocalExecutionIdentity,
) -> LocalExecutionIdentityGuard {
    let key = LOCAL_EXECUTION.with(|stack| stack.borrow_mut().push(identity));
    LocalExecutionIdentityGuard { key, _same_thread: PhantomData }
}

pub(crate) fn current_local_execution_identity() -> Option<LocalExecutionIdentity> {
    LOCAL_EXECUTION.with(|stack| stack.borrow().current().cloned())
}

impl Drop for LocalExecutionIdentityGuard {
    fn drop(&mut self) {
        LOCAL_EXECUTION.with(|stack| stack.borrow_mut().remove(self.key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(nonce: &str) -> LocalExecutionIdentity {
        LocalExecutionIdentity {
            execution_id: nonce.into(), host_id: "host".into(), session_id: "session".into(),
            turn_id: "turn".into(), effect_id: "effect".into(), request_id: "request".into(),
            call_id: "call".into(), delivery_attempt_id: None,
        }
    }

    #[test]
    fn private_stop_identity_is_thread_local_and_cannot_resurrect_a_dropped_owner() {
        assert!(current_local_execution_identity().is_none());
        let first = enter_local_execution_identity(identity("first"));
        let second = enter_local_execution_identity(identity("second"));
        assert_eq!(current_local_execution_identity(), Some(identity("second")));
        std::thread::spawn(|| assert!(current_local_execution_identity().is_none())).join().unwrap();
        drop(first);
        assert_eq!(current_local_execution_identity(), Some(identity("second")));
        drop(second);
        assert!(current_local_execution_identity().is_none());
    }
}
