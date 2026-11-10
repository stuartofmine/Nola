// SPDX-License-Identifier: Apache-2.0
//! Execution-local control and input ownership. No terminal, renderer or draft.
//! Application owns durable admission and ACK; this handle never invents them.
use crate::execution_context::{ExecutionContext, ExecutionContextScopeGuard, ScopeStack};
use crate::runtime_input::{DurableInputPort, InputReceipt, RuntimeInput, RuntimeInputMailbox};
use std::cell::RefCell;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

thread_local! {
    static CURRENT_CONTROL: RefCell<ScopeStack<RuntimeExecutionControl>> = const { RefCell::new(ScopeStack::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputDisposition {
    ActiveBoundary,
    NextTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputRetention { Persistent, Process }

impl InputRetention {
    pub(crate) fn for_session(save_session: bool) -> Self {
        if save_session { Self::Persistent } else { Self::Process }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionInputOwner { session_id: String, retention: InputRetention }

pub(crate) trait UserInstructionHandoff: Send + Sync + std::fmt::Debug {
    fn handoff(&self, text: String) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct RuntimeExecutionControl {
    context: ExecutionContext,
    background_requested: Arc<AtomicBool>,
    background_transfer_enabled: bool,
    mailbox: Arc<Mutex<RuntimeInputMailbox>>,
    input_owner: Arc<Mutex<Option<SessionInputOwner>>>,
    input_disposition: InputDisposition,
    instruction_handoff: Option<Arc<dyn UserInstructionHandoff>>,
    interaction: Arc<dyn crate::runtime_interaction::RuntimeInteraction>,
}

impl Default for RuntimeExecutionControl {
    fn default() -> Self {
        Self {
            context: ExecutionContext::default(),
            background_requested: Arc::new(AtomicBool::new(false)),
            background_transfer_enabled: false,
            mailbox: Arc::new(Mutex::new(RuntimeInputMailbox::default())),
            input_owner: Arc::new(Mutex::new(None)),
            input_disposition: InputDisposition::ActiveBoundary,
            instruction_handoff: None,
            interaction: Arc::new(crate::runtime_interaction::UnavailableInteraction),
        }
    }
}

struct ApplicationInputPort;
impl DurableInputPort for ApplicationInputPort {
    fn admit(&mut self, session: &str, text: &str) -> Result<(), String> {
        crate::boundary_input_intent::execute(
            session,
            crate::boundary_input_intent::InputOperation::NewSubmit(text),
        )
        .map(|_| ())
    }
}

impl RuntimeExecutionControl {
    pub fn with_interaction(
        mut self,
        interaction: Arc<dyn crate::runtime_interaction::RuntimeInteraction>,
    ) -> Self {
        self.interaction = interaction;
        self
    }
    pub fn interaction(&self) -> Arc<dyn crate::runtime_interaction::RuntimeInteraction> {
        self.interaction.clone()
    }
    pub(crate) fn with_input_adapter(
        mut self,
        mailbox: Arc<Mutex<RuntimeInputMailbox>>,
        disposition: InputDisposition,
        handoff: Arc<dyn UserInstructionHandoff>,
    ) -> Self {
        self.mailbox = mailbox;
        self.input_disposition = disposition;
        self.instruction_handoff = Some(handoff);
        self
    }
    pub(crate) fn with_background_transfer(mut self) -> Self {
        self.background_transfer_enabled = true;
        self
    }
    pub fn context(&self) -> ExecutionContext {
        self.context.clone()
    }
    pub fn execution_id(&self) -> u64 {
        self.context.execution_id()
    }
    pub fn request(&self) {
        self.context.request();
    }
    pub fn is_requested(&self) -> bool {
        self.context.is_requested()
    }
    pub fn request_background(&self) -> bool {
        if !self.background_transfer_enabled || self.is_requested() {
            return false;
        }
        self.background_requested.store(true, Ordering::SeqCst);
        true
    }
    pub fn is_background_requested(&self) -> bool {
        self.background_transfer_enabled && self.background_requested.load(Ordering::SeqCst)
    }
    pub fn take_background_request(&self) -> bool {
        self.background_transfer_enabled && self.background_requested.swap(false, Ordering::SeqCst)
    }
    pub fn supports_background_transfer(&self) -> bool {
        self.background_transfer_enabled
    }
    pub fn child(&self) -> Self {
        Self {
            context: self.context.child(),
            ..self.clone()
        }
    }

    /// An owner cannot silently rebind pending inputs into another conversation.
    pub(crate) fn bind_session_input(&self, session: impl Into<String>, retention: InputRetention) -> Result<(), String> {
        let session = session.into();
        if session.trim().is_empty() {
            return Err("execution input requires a session owner".into());
        }
        let mut owner = self
            .input_owner
            .lock()
            .map_err(|_| "execution input owner poisoned")?;
        let next = SessionInputOwner { session_id: session, retention };
        match owner.as_ref() {
            Some(existing) if existing != &next => {
                Err("execution input owner cannot change".into())
            }
            _ => {
                *owner = Some(next);
                Ok(())
            }
        }
    }
    pub(crate) fn queue_runtime_input(&self, input: RuntimeInput) -> Result<InputReceipt, String> {
        let owner = self
            .input_owner
            .lock()
            .map_err(|_| "execution input owner poisoned")?;
        let mut mailbox = self
            .mailbox
            .lock()
            .map_err(|_| "execution input mailbox poisoned")?;
        crate::runtime_input::deliver(
            input,
            owner.as_ref().filter(|owner| owner.retention == InputRetention::Persistent)
                .map(|owner| owner.session_id.as_str()),
            self.is_requested(),
            &mut mailbox,
            &mut ApplicationInputPort,
        )
    }
    pub(crate) fn enqueue_local(&self, input: RuntimeInput) -> Result<InputReceipt, String> {
        self.mailbox
            .lock()
            .map_err(|_| "execution input mailbox poisoned")?
            .enqueue(input)
    }
    pub fn take_queued_inputs(&self) -> Vec<String> {
        self.mailbox
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain_all()
    }
    pub fn take_boundary_user_inputs(&self) -> Vec<String> {
        if self.is_requested() || self.input_disposition == InputDisposition::NextTurn {
            return vec![];
        }
        self.mailbox
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain_followups()
    }
    /// Permission UI hands authored text back to its owner, never to the model
    /// implicitly. Headless clients retain an explicit next-input queue entry.
    pub fn handoff_user_instruction(&self, text: String) -> Result<(), String> {
        self.request();
        if let Some(handoff) = &self.instruction_handoff {
            handoff.handoff(text)
        } else {
            self.enqueue_local(RuntimeInput::UserFollowUp(text))
                .map(|_| ())
        }
    }
    pub fn enter(&self) -> RuntimeControlScope {
        let context = self.context.enter();
        let binding = CURRENT_CONTROL.with(|value| value.borrow_mut().push(self.clone()));
        RuntimeControlScope {
            _context: context,
            binding,
        }
    }
}

pub struct RuntimeControlScope {
    _context: ExecutionContextScopeGuard,
    binding: u64,
}
impl Drop for RuntimeControlScope {
    fn drop(&mut self) {
        CURRENT_CONTROL.with(|value| value.borrow_mut().remove(self.binding));
    }
}
pub(crate) fn current_execution_control() -> Option<RuntimeExecutionControl> {
    CURRENT_CONTROL.with(|value| value.borrow().current().cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_cancel_is_local_but_input_and_owner_remain_shared() {
        let root = RuntimeExecutionControl::default();
        root.bind_session_input("owner", InputRetention::Persistent).unwrap();
        let first = root.child();
        let sibling = root.child();
        first.request();
        assert!(!root.is_requested());
        assert!(!sibling.is_requested());
        assert!(sibling.bind_session_input("other", InputRetention::Persistent).is_err());
        first
            .queue_runtime_input(RuntimeInput::UserFollowUp("after cancel".into()))
            .unwrap();
        assert!(first.take_boundary_user_inputs().is_empty());
        assert_eq!(root.take_queued_inputs(), vec!["after cancel"]);
        assert!(sibling.take_queued_inputs().is_empty());
        root.request();
        assert!(sibling.is_requested());
    }

    #[test]
    fn process_retained_session_keeps_followups_local_without_losing_owner_or_next_turn_policy() {
        let _environment = crate::store::test_env_lock();
        let fixture = tempfile::tempdir().unwrap();
        let database = fixture.path().join("must-not-be-created.sqlite");
        struct RestoreDatabase(Option<std::ffi::OsString>);
        impl Drop for RestoreDatabase {
            fn drop(&mut self) {
                if let Some(value) = &self.0 { std::env::set_var("NOLA_DB_PATH", value); }
                else { std::env::remove_var("NOLA_DB_PATH"); }
            }
        }
        let _restore = RestoreDatabase(std::env::var_os("NOLA_DB_PATH"));
        std::env::set_var("NOLA_DB_PATH", &database);
        let control = RuntimeExecutionControl {
            input_disposition: InputDisposition::NextTurn,
            ..RuntimeExecutionControl::default()
        };
        control.bind_session_input("ephemeral", InputRetention::Process).unwrap();
        let child = control.child();
        assert!(child.bind_session_input("foreign", InputRetention::Process).is_err());
        assert!(child.bind_session_input("ephemeral", InputRetention::Persistent).is_err());
        assert_eq!(control.queue_runtime_input(RuntimeInput::UserFollowUp("next turn".into())).unwrap(), InputReceipt::Local);
        assert!(control.take_boundary_user_inputs().is_empty());
        assert_eq!(control.take_queued_inputs(), vec!["next turn"]);
        assert!(child.take_queued_inputs().is_empty());
        assert!(!database.exists(), "process input must not open a persistent Application");
        assert!(!fixture.path().join("client-input-intents").exists(),
            "process input must not create a durable recovery journal");
    }

    #[test]
    fn nested_scopes_restore_owner_even_after_error() {
        let parent = RuntimeExecutionControl::default();
        let child = parent.child();
        let _parent = parent.enter();
        let fail = || -> Result<(), String> {
            let _child = child.enter();
            assert_eq!(
                current_execution_control().unwrap().execution_id(),
                child.execution_id()
            );
            Err("early return".into())
        };
        assert!(fail().is_err());
        assert_eq!(
            current_execution_control().unwrap().execution_id(),
            parent.execution_id()
        );
    }

    #[test]
    fn headless_instruction_handoff_never_becomes_live_input() {
        let control = RuntimeExecutionControl::default();
        control
            .handoff_user_instruction("User clarification".into())
            .unwrap();
        assert!(control.is_requested());
        assert!(control.take_boundary_user_inputs().is_empty());
        assert_eq!(control.take_queued_inputs(), vec!["User clarification"]);
    }

    #[test]
    fn typed_followup_and_control_have_no_keyword_routing() {
        let control = RuntimeExecutionControl::default();
        control
            .queue_runtime_input(RuntimeInput::UserFollowUp(
                "/literal source text 搜索 current".into(),
            ))
            .unwrap();
        control
            .queue_runtime_input(RuntimeInput::ClientControl("continue".into()))
            .unwrap();
        assert_eq!(
            control.take_boundary_user_inputs(),
            vec!["/literal source text 搜索 current"]
        );
        assert_eq!(control.take_queued_inputs(), vec!["continue"]);
    }

    #[test]
    fn background_transfer_requires_explicit_capability_and_is_shared_by_children() {
        let control = RuntimeExecutionControl::default();
        assert!(!control.supports_background_transfer());
        let child = control.child();
        assert!(!child.request_background());
        assert!(!control.supports_background_transfer());
        assert!(!control.take_background_request());
        let control = control.with_background_transfer();
        let child = control.child();
        assert!(control.supports_background_transfer());
        assert!(child.request_background());
        assert!(control.take_background_request());
        assert!(!child.is_background_requested());
        control.request();
        assert!(!child.request_background());
    }
}

impl nola_mcp_host::Cancellation for RuntimeExecutionControl {
    fn is_requested(&self) -> bool { RuntimeExecutionControl::is_requested(self) }
}
