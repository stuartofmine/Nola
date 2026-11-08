// SPDX-License-Identifier: Apache-2.0
//! Nonblocking admission for connection workers, including projection waiters.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

pub(super) struct ConnectionBudget {
    active: Arc<AtomicUsize>,
    limit: usize,
}

pub(super) struct ConnectionPermit(Arc<AtomicUsize>);

impl ConnectionBudget {
    pub(super) fn for_process() -> Self {
        let mut limits = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        let limit = if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) } == 0 {
            // Reserve descriptors for persistence, sockets and runtime effects;
            // retain headroom per admitted connection for projection waiters.
            descriptor_limit(limits.rlim_cur as u64)
        } else {
            32
        };
        Self::new(limit)
    }

    pub(super) fn new(limit: usize) -> Self {
        Self {
            active: Arc::new(AtomicUsize::new(0)),
            limit,
        }
    }

    pub(super) fn try_acquire(&self) -> Option<ConnectionPermit> {
        self.active
            .try_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < self.limit).then(|| active + 1)
            })
            .ok()
            .map(|_| ConnectionPermit(Arc::clone(&self.active)))
    }
}

fn descriptor_limit(soft_limit: u64) -> usize {
    (soft_limit.saturating_sub(64) / 2).clamp(1, 128) as usize
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_reserves_descriptor_headroom() {
        assert_eq!(descriptor_limit(256), 96);
        assert_eq!(descriptor_limit(128), 32);
        assert_eq!(descriptor_limit(64), 1);
        assert_eq!(descriptor_limit(u64::MAX), 128);
    }

    #[test]
    fn permits_are_bounded_and_released_on_unwind() {
        let budget = ConnectionBudget::new(1);
        let _ = std::panic::catch_unwind(|| {
            let _permit = budget.try_acquire().unwrap();
            assert!(budget.try_acquire().is_none());
            panic!("fixture worker failure");
        });
        assert!(budget.try_acquire().is_some());
    }

    #[test]
    fn simultaneous_admission_never_exceeds_limit() {
        let budget = Arc::new(ConnectionBudget::new(4));
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let workers: Vec<_> = (0..16)
            .map(|_| {
                let budget = Arc::clone(&budget);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let permit = budget.try_acquire();
                    barrier.wait();
                    let admitted = permit.is_some();
                    barrier.wait();
                    admitted
                })
            })
            .collect();
        let admitted = workers
            .into_iter()
            .map(|worker| usize::from(worker.join().unwrap()))
            .sum::<usize>();
        assert_eq!(admitted, 4);
        assert_eq!(budget.active.load(Ordering::Acquire), 0);
    }
}
