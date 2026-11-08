// SPDX-License-Identifier: Apache-2.0
//! Process signals are cancellation requests, never durable terminal facts.
//! The execution owner keeps this guard until Core commits its real outcome.
use crate::runtime_control::RuntimeExecutionControl;

pub(crate) struct ExecutionSignals {
    handle: signal_hook::iterator::Handle,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ExecutionSignals {
    pub(crate) fn install(
        control: RuntimeExecutionControl,
        wake_waiters: Option<fn()>,
    ) -> Result<Self, String> {
        let mut signals = signal_hook::iterator::Signals::new([
            signal_hook::consts::signal::SIGINT,
            signal_hook::consts::signal::SIGTERM,
        ])
        .map_err(|error| format!("failed to install execution cancellation signals: {error}"))?;
        let handle = signals.handle();
        let worker = std::thread::Builder::new()
            .name("execution-signals".into())
            .spawn(move || {
                for _ in signals.forever() {
                    control.request();
                    if let Some(wake) = wake_waiters {
                        wake();
                    }
                }
            })
            .map_err(|error| format!("failed to start execution cancellation signals: {error}"))?;
        Ok(Self {
            handle,
            worker: Some(worker),
        })
    }
}

impl Drop for ExecutionSignals {
    fn drop(&mut self) {
        // Close wakes the iterator even if no signal ever arrived. Joining
        // prevents a late signal from cancelling a subsequent execution.
        self.handle.close();
        if let Some(worker) = self.worker.take() {
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
