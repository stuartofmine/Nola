// SPDX-License-Identifier: Apache-2.0
//! Process signals request cancellation; Core commits the terminal outcome.
use crate::runtime_control::RuntimeExecutionControl;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

pub(crate) struct ExecutionSignals {
    registrations: Vec<signal_hook::SigId>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ExecutionSignals {
    pub(crate) fn install(control: RuntimeExecutionControl, wake_waiters: Option<fn()>) -> Result<Self, String> {
        Self::install_callback(move || {
            control.request();
            if let Some(wake) = wake_waiters { wake(); }
        })
    }

    /// signal-hook's iterator is Unix-only. Flag registration also supports
    /// Windows and keeps allocation, locking and callbacks out of signal handlers.
    pub(crate) fn install_callback(mut callback: impl FnMut() + Send + 'static) -> Result<Self, String> {
        let pending = Arc::new(AtomicBool::new(false));
        let mut guard = Self { registrations: vec![], stop: Arc::new(AtomicBool::new(false)), worker: None };
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
            guard.registrations.push(signal_hook::flag::register(signal, pending.clone())
                .map_err(|error| format!("failed to install execution cancellation signals: {error}"))?);
        }
        let stop = guard.stop.clone();
        guard.worker = Some(std::thread::Builder::new().name("execution-signals".into()).spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if pending.swap(false, Ordering::AcqRel) { callback(); }
                std::thread::park_timeout(std::time::Duration::from_millis(20));
            }
        }).map_err(|error| format!("failed to start execution cancellation signals: {error}"))?);
        Ok(guard)
    }
}

impl Drop for ExecutionSignals {
    fn drop(&mut self) {
        for id in self.registrations.drain(..) { signal_hook::low_level::unregister(id); }
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_listener_is_cleaned_up_without_cancelling_the_run() {
        for _ in 0..3 {
            let control = RuntimeExecutionControl::default();
            drop(ExecutionSignals::install(control.clone(), None).unwrap());
            assert!(!control.is_requested());
        }
    }
}
