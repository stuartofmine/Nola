// SPDX-License-Identifier: Apache-2.0
//! Cancellation identity and lexical thread-scope contracts; no terminal or IO.

use super::{current_execution_id, ExecutionContext, ExecutionContextScopeGuard};
use std::collections::BTreeSet;
use std::sync::mpsc;

#[test]
fn external_supervisor_budget_is_measured_not_recovered_from_task_text() {
    let budget = super::ExternalExecutionBudget::parse("1000000", "600").unwrap();
    let early = budget.message_at(500000);
    let late = budget.message_at(950000);
    assert_eq!(early["role"], "user");
    for (message, remaining) in [(&early, 500000), (&late, 50000)] {
        let content = message["content"].as_str().unwrap();
        let line = content.lines().find(|line| line.starts_with('{')).unwrap();
        let facts: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(facts["remaining_ms"], remaining);
        assert_eq!(facts["time_limit_seconds"], 600.0);
        assert_eq!(facts["deadline_unix_ms"], 1000000);
        assert!(content.contains("not a per-tool timeout"));
        assert!(content.contains("or evidence that any Goal is complete"));
    }
    let expired = budget.message_at(1000001).to_string();
    assert!(expired.contains("\\\"remaining_ms\\\":0"));
    assert!(expired.contains("\\\"expired\\\":true"));
    for (deadline, limit) in [("", "600"), ("0", "600"), ("1000", "NaN"),
        ("1000", "inf"), ("1000", "-1"), ("1000", "0"), ("instruction", "600")] {
        assert!(super::ExternalExecutionBudget::parse(deadline, limit).is_none());
    }
}

#[test]
fn dropping_scopes_out_of_order_never_revives_expired_identity() {
    let outer = ExecutionContext::default();
    let middle = ExecutionContext::default();
    let inner = ExecutionContext::default();
    let a = outer.enter();
    let b = middle.enter();
    let c = inner.enter();
    drop(b);
    assert_eq!(current_execution_id(), Some(inner.execution_id()));
    drop(c);
    assert_eq!(current_execution_id(), Some(outer.execution_id()));
    let d = outer.enter();
    drop(a);
    assert_eq!(current_execution_id(), Some(outer.execution_id()));
    drop(d);
    assert_eq!(current_execution_id(), None);
}

#[test]
fn clones_share_cancellation_and_identity_but_independent_runs_do_not() {
    let first = ExecutionContext::default();
    let clone = first.clone();
    let second = ExecutionContext::default();
    assert_eq!(first.execution_id(), clone.execution_id());
    assert_ne!(first.execution_id(), second.execution_id());
    clone.request();
    clone.request();
    assert!(first.is_requested());
    assert!(clone.is_requested());
    assert!(!second.is_requested());
}

#[test]
fn cancellation_flows_down_all_ancestors_never_up_or_sideways() {
    let root = ExecutionContext::default();
    let middle = root.child();
    let leaf = middle.child();
    let sibling = root.child();
    let sibling_leaf = sibling.child();
    let identities = [&root, &middle, &leaf, &sibling, &sibling_leaf]
        .map(ExecutionContext::execution_id)
        .into_iter()
        .collect::<BTreeSet<_>>();
    assert_eq!(identities.len(), 5);

    leaf.request();
    assert!(leaf.is_requested());
    assert!(!middle.is_requested());
    assert!(!root.is_requested());
    assert!(!sibling.is_requested());
    middle.request();
    assert!(
        middle.child().is_requested(),
        "late children must observe prior cancellation"
    );
    assert!(!root.is_requested());
    assert!(!sibling_leaf.is_requested());
    root.request();
    assert!(sibling.is_requested());
    assert!(sibling_leaf.is_requested());
    assert!(root.child().child().is_requested());
}

#[test]
fn descendants_retain_ancestor_cancellation_after_parent_handles_are_dropped() {
    let root = ExecutionContext::default();
    let root_requester = root.clone();
    let middle = root.child();
    let leaf = middle.child();
    drop(middle);
    drop(root);
    assert!(!leaf.is_requested());
    root_requester.request();
    drop(root_requester);
    assert!(leaf.is_requested());
}

#[test]
fn nested_scopes_restore_outer_identity_without_cancelling_tokens() {
    assert_eq!(current_execution_id(), None);
    let outer = ExecutionContext::default();
    let inner = outer.child();
    {
        let _outer_guard = outer.enter();
        assert_eq!(current_execution_id(), Some(outer.execution_id()));
        {
            let _inner_guard = inner.enter();
            assert_eq!(current_execution_id(), Some(inner.execution_id()));
            {
                let _same_context_guard = inner.enter();
                assert_eq!(current_execution_id(), Some(inner.execution_id()));
            }
            assert_eq!(current_execution_id(), Some(inner.execution_id()));
        }
        assert_eq!(current_execution_id(), Some(outer.execution_id()));
    }
    assert_eq!(current_execution_id(), None);
    assert!(!outer.is_requested());
    assert!(!inner.is_requested());
}

#[test]
fn scope_unwinding_restores_prior_thread_identity() {
    let outer = ExecutionContext::default();
    let inner = outer.child();
    {
        let _outer_guard = outer.enter();
        let result = std::panic::catch_unwind(|| {
            let _inner_guard = inner.enter();
            assert_eq!(current_execution_id(), Some(inner.execution_id()));
            panic!("fixture unwind");
        });
        assert!(result.is_err());
        assert_eq!(current_execution_id(), Some(outer.execution_id()));
    }
    assert_eq!(current_execution_id(), None);
}

#[test]
fn worker_scopes_are_explicit_thread_local_and_cancellation_is_shared() {
    let parent = ExecutionContext::default();
    let child = parent.child();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (cancelled_tx, cancelled_rx) = mpsc::channel();
    {
        let _parent_guard = parent.enter();
        let worker = std::thread::spawn(move || {
            assert_eq!(
                current_execution_id(),
                None,
                "spawn must not inherit ambient thread identity"
            );
            {
                let _worker_guard = child.enter();
                assert_eq!(current_execution_id(), Some(child.execution_id()));
                ready_tx.send(()).unwrap();
                cancelled_rx.recv().unwrap();
                assert!(child.is_requested());
                assert_eq!(current_execution_id(), Some(child.execution_id()));
            }
            assert_eq!(current_execution_id(), None);
        });
        ready_rx.recv().unwrap();
        assert_eq!(current_execution_id(), Some(parent.execution_id()));
        parent.request();
        cancelled_tx.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(current_execution_id(), Some(parent.execution_id()));
    }
    assert_eq!(current_execution_id(), None);
}

#[test]
fn parallel_context_creation_never_reuses_an_execution_identity() {
    let workers = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                (0..32)
                    .map(|_| ExecutionContext::default().execution_id())
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    let ids = workers
        .into_iter()
        .flat_map(|worker| worker.join().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 256);
}

#[test]
fn cancellation_handles_cross_threads_but_scope_guards_cannot() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ExecutionContext>();

    // If the guard implements Send this call becomes ambiguous and compilation
    // fails. Moving a guard would restore another thread's TLS on Drop.
    trait AmbiguousIfSend<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    struct Invalid;
    impl<T: ?Sized + Send> AmbiguousIfSend<Invalid> for T {}
    let _ = <ExecutionContextScopeGuard as AmbiguousIfSend<_>>::check;
}
