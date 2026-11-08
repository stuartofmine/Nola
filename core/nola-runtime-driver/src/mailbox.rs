// SPDX-License-Identifier: Apache-2.0
//! Bounded, non-durable physical Host transport. Only the committed driver
//! acknowledges responses. Closing loses transport, never proves execution stopped.
use nola_host_protocol::{
    HostDriver, HostExecutionProtocol, HostRequestEnvelope, HostResponseEnvelope,
    RuntimeResponsePayload,
};
use std::collections::{HashMap, VecDeque};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Condvar, Mutex,
};
use std::time::{Duration, Instant};

type Request<P> = HostRequestEnvelope<<P as HostExecutionProtocol>::Request>;
type Response<P> = HostResponseEnvelope<<P as HostExecutionProtocol>::Response>;
struct ResponseDelivery<P: HostExecutionProtocol> {
    response: Response<P>,
    ack: mpsc::SyncSender<Result<(), String>>,
}
struct ReplyPort<P: HostExecutionProtocol> {
    sender: mpsc::SyncSender<ResponseDelivery<P>>,
    busy: AtomicBool,
}
struct State<P: HostExecutionProtocol> {
    next_ticket: u64,
    closed: bool,
    capacity: usize,
    pending: VecDeque<(u64, Request<P>)>,
    replies: HashMap<u64, Arc<ReplyPort<P>>>,
}
struct Shared<P: HostExecutionProtocol> {
    state: Mutex<State<P>>,
    ready: Condvar,
}
pub struct MailboxHostDriver<P: HostExecutionProtocol> {
    shared: Arc<Shared<P>>,
}
pub struct HostMailbox<P: HostExecutionProtocol> {
    shared: Arc<Shared<P>>,
}
impl<P: HostExecutionProtocol> Clone for HostMailbox<P> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

pub fn bounded_host_mailbox<P: HostExecutionProtocol>(
    capacity: usize,
) -> Result<(MailboxHostDriver<P>, HostMailbox<P>), String> {
    if !(1..=64).contains(&capacity) {
        return Err("Host mailbox capacity must be 1...64".into());
    }
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            next_ticket: 1,
            closed: false,
            capacity,
            pending: VecDeque::new(),
            replies: HashMap::new(),
        }),
        ready: Condvar::new(),
    });
    Ok((
        MailboxHostDriver {
            shared: shared.clone(),
        },
        HostMailbox { shared },
    ))
}
impl<P: HostExecutionProtocol> HostMailbox<P> {
    /// Returns each claimed request once. Timeout is an idle poll, not permission
    /// to claim again, retry a request, or infer terminal state.
    pub fn poll(&self, wait: Duration) -> Result<Option<(u64, Request<P>)>, String> {
        if wait > Duration::from_secs(30) {
            return Err("Host mailbox poll exceeds 30 seconds".into());
        }
        let until = Instant::now() + wait;
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| "Host mailbox lock poisoned")?;
        loop {
            if state.closed {
                return Err("Host mailbox closed; execution outcome may be uncertain".into());
            }
            if let Some(request) = state.pending.pop_front() {
                return Ok(Some(request));
            }
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            state = self
                .shared
                .ready
                .wait_timeout(state, remaining)
                .map_err(|_| "Host mailbox lock poisoned")?
                .0;
        }
    }
    /// The caller MUST wait for this commit ACK before starting physical work
    /// after ExecutionStarted, or advancing any response sequence. There is no
    /// caller-supplied ACK API and no acknowledgement on enqueue.
    pub fn respond(&self, ticket: u64, response: Response<P>) -> Result<(), String> {
        let port = {
            let state = self
                .shared
                .state
                .lock()
                .map_err(|_| "Host mailbox lock poisoned")?;
            if state.closed {
                return Err("Host mailbox closed; response outcome unconfirmed".into());
            }
            state
                .replies
                .get(&ticket)
                .cloned()
                .ok_or("unknown or settled Host mailbox ticket")?
        };
        if port.busy.swap(true, Ordering::AcqRel) {
            return Err("Host response is awaiting its commit ACK".into());
        }
        struct Reset<'a>(&'a AtomicBool);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _reset = Reset(&port.busy);
        let (ack, received) = mpsc::sync_channel(1);
        port.sender
            .try_send(ResponseDelivery { response, ack })
            .map_err(|_| "Host response mailbox unavailable; do not redispatch")?;
        loop {
            match received.recv_timeout(Duration::from_millis(100)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Host response commit ACK lost; outcome unconfirmed".into())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self
                        .shared
                        .state
                        .lock()
                        .map_err(|_| "Host mailbox lock poisoned")?
                        .closed
                    {
                        return Err(
                            "Host mailbox closed while awaiting commit ACK; outcome unconfirmed"
                                .into(),
                        );
                    }
                }
            }
        }
    }
    pub fn close(&self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.closed = true;
            state.pending.clear();
            state.replies.clear();
            self.shared.ready.notify_all();
        }
    }
}
impl<P: HostExecutionProtocol> HostDriver for MailboxHostDriver<P>
where
    P::Request: Clone,
{
    type Protocol = P;
    type Error = String;
    fn dispatch(
        &mut self,
        request: Request<P>,
        emit: &mut dyn FnMut(Response<P>) -> Result<(), String>,
    ) -> Result<(), String> {
        let expected = request.clone();
        let started = request.attempt_started;
        let (sender, receiver) = mpsc::sync_channel(1);
        let ticket = {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| "Host mailbox lock poisoned")?;
            if state.closed {
                return Err("Host mailbox closed before delivery".into());
            }
            if state.replies.len() >= state.capacity {
                return Err("Host mailbox capacity exhausted before delivery".into());
            }
            let ticket = state.next_ticket;
            state.next_ticket = ticket
                .checked_add(1)
                .ok_or("Host mailbox ticket exhausted")?;
            state.replies.insert(
                ticket,
                Arc::new(ReplyPort {
                    sender,
                    busy: AtomicBool::new(false),
                }),
            );
            state.pending.push_back((ticket, request));
            self.shared.ready.notify_all();
            ticket
        };
        let result = (|| {
            let mut started = started;
            loop {
                let delivery = match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(value) => value,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if self
                            .shared
                            .state
                            .lock()
                            .map_err(|_| "Host mailbox lock poisoned")?
                            .closed
                        {
                            return Err(
                                "Host mailbox disconnected; execution outcome remains uncertain"
                                    .into(),
                            );
                        }
                        continue;
                    }
                    Err(_) => {
                        return Err(
                            "Host responder disconnected; execution outcome remains uncertain"
                                .into(),
                        )
                    }
                };
                let is_start = delivery.response.payload.execution_started();
                let terminal = delivery.response.payload.is_terminal();
                let yielded = delivery
                    .response
                    .payload
                    .permission_suspension_id()
                    .is_some()
                    || delivery
                        .response
                        .payload
                        .code_cell_suspension_id()
                        .is_some();
                // Pre-start terminal rejections are validated by Application;
                // incremental progress cannot precede the exact start ACK.
                let response = &delivery.response;
                let exact = response.request_id == expected.request_id
                    && response.effect_id == expected.effect_id
                    && response.host_id == expected.host_id
                    && response.lease_id == expected.lease_id
                    && response.session_id == expected.session_id
                    && response.turn_id == expected.turn_id
                    && response.attempt_id == expected.attempt_id
                    && response.delegated_invocation_id == expected.delegated_invocation_id
                    && response.catalog_revision == expected.catalog_revision
                    && response.availability_epoch == expected.availability_epoch
                    && response.delivery == expected.delivery;
                let accepted = if !exact {
                    Err("Host response belongs to another mailbox request or delivery fence".into())
                } else if !started && !is_start && !terminal {
                    Err("Host progress requires a committed ExecutionStarted ACK".into())
                } else {
                    emit(delivery.response)
                };
                let committed = accepted.is_ok();
                let failure = accepted.as_ref().err().cloned();
                let ack_delivered = delivery.ack.send(accepted).is_ok();
                if let Some(error) = failure {
                    return Err(error);
                }
                if !ack_delivered {
                    return Err("Host commit ACK receiver disconnected; do not redispatch".into());
                }
                started |= committed && is_start;
                if terminal || yielded {
                    return Ok(());
                }
            }
        })();
        if let Ok(mut state) = self.shared.state.lock() {
            state.replies.remove(&ticket);
            state.pending.retain(|(id, _)| *id != ticket);
        }
        result
    }
}
