// SPDX-License-Identifier: Apache-2.0
//! UI-independent execution input delivery. Application owns durable FIFO/ACK;
//! this mailbox only owns process-local, explicitly typed inputs.
use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeInput {
    UserFollowUp(String),
    ClientControl(String),
}
impl RuntimeInput {
    fn text(&self) -> &str { match self { Self::UserFollowUp(text) | Self::ClientControl(text) => text } }
    fn into_text(self) -> String { match self { Self::UserFollowUp(text) | Self::ClientControl(text) => text } }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputReceipt { Durable, Local }

pub(crate) trait DurableInputPort {
    /// Return success only after Application has committed its existing FIFO
    /// entry. This is producer admission, not consumer acknowledgement.
    fn admit(&mut self, session: &str, text: &str) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub(crate) struct RuntimeInputMailbox { queued: VecDeque<RuntimeInput> }
impl RuntimeInputMailbox {
    pub(crate) fn len(&self) -> usize { self.queued.len() }
    pub(crate) fn enqueue(&mut self, input: RuntimeInput) -> Result<InputReceipt, String> {
        if input.text().trim().is_empty() { return Err("empty execution input".into()); }
        if self.queued.len() >= 32 { return Err("execution input queue full".into()); }
        self.queued.push_back(input); Ok(InputReceipt::Local)
    }
    pub(crate) fn drain_all(&mut self) -> Vec<String> {
        self.queued.drain(..).map(RuntimeInput::into_text).collect()
    }
    pub(crate) fn drain_followups(&mut self) -> Vec<String> {
        let mut followups = Vec::new();
        self.queued.retain(|input| match input {
            RuntimeInput::UserFollowUp(text) => { followups.push(text.clone()); false }
            RuntimeInput::ClientControl(_) => true,
        });
        followups
    }
}

/// No text routing, key events, draft mutation, terminal ownership or new
/// durable queue. Cancellation/deferred-turn policy is supplied by the owner.
pub(crate) fn deliver(
    input: RuntimeInput,
    durable_session: Option<&str>,
    cancelled: bool,
    local: &mut RuntimeInputMailbox,
    durable: &mut impl DurableInputPort,
) -> Result<InputReceipt, String> {
    if input.text().trim().is_empty() { return Err("empty execution input".into()); }
    if let (RuntimeInput::UserFollowUp(text), Some(session), false) = (&input, durable_session, cancelled) {
        durable.admit(session, text)?;
        return Ok(InputReceipt::Durable);
    }
    local.enqueue(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Port { fail: bool, calls: Vec<(String, String)> }
    impl DurableInputPort for Port {
        fn admit(&mut self, session: &str, text: &str) -> Result<(), String> {
            self.calls.push((session.into(), text.into()));
            if self.fail { Err("unconfirmed".into()) } else { Ok(()) }
        }
    }
    #[test]
    fn durable_failure_never_becomes_local_success_or_duplicate() {
        let mut port = Port { fail:true, calls:vec![] }; let mut local=RuntimeInputMailbox::default();
        assert!(deliver(RuntimeInput::UserFollowUp("/literal model text".into()),Some("session"),false,&mut local,&mut port).is_err());
        assert_eq!(local.len(),0); assert_eq!(port.calls.len(),1);
    }
    #[test]
    fn typed_controls_and_cancellation_preserve_fifo_without_durable_dispatch() {
        let mut port=Port { fail:false,calls:vec![] }; let mut local=RuntimeInputMailbox::default();
        deliver(RuntimeInput::ClientControl("/exit".into()),Some("session"),false,&mut local,&mut port).unwrap();
        deliver(RuntimeInput::UserFollowUp("after cancellation".into()),Some("session"),true,&mut local,&mut port).unwrap();
        assert!(port.calls.is_empty());
        assert_eq!(local.drain_followups(),vec!["after cancellation"]);
        assert_eq!(local.drain_all(),vec!["/exit"]);
    }
}
