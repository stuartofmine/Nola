// SPDX-License-Identifier: Apache-2.0
//! Host interaction port: gathers user choices, never evaluates or grants Core permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPromptKind {
    Command,
    Tool,
    Network,
    Agent,
}

#[derive(Debug, Clone)]
pub struct PermissionPrompt {
    pub allow_conversation: bool,
    pub kind: PermissionPromptKind,
    pub title: String,
    pub rows: Vec<(String, String)>,
    pub session_label: String,
    pub session_value: String,
    pub custom_label: String,
    pub supports_instruction: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionChoice {
    TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason),
    Once,
    Session(String),
    Custom(String),
    ReturnToComposer,
    Deny,
}

#[derive(Debug, Clone)]
pub struct UserQuestionPrompt {
    pub header: String,
    pub question: String,
    pub options: Vec<(String, String)>,
    pub allow_freeform: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserQuestionAnswer {
    Selected { index: usize, label: String },
    Freeform(String),
    Cancelled,
    Declined,
    Unavailable,
}

pub trait RuntimeInteraction: std::fmt::Debug + Send + Sync {
    fn permission_available(&self) -> bool;
    fn request_permission(&self, prompt: &PermissionPrompt) -> PermissionChoice;
    fn request_user_question(&self, prompt: &UserQuestionPrompt) -> UserQuestionAnswer;
}

#[derive(Debug, Default)]
pub struct UnavailableInteraction;
impl RuntimeInteraction for UnavailableInteraction {
    fn permission_available(&self) -> bool {
        false
    }
    fn request_permission(&self, _: &PermissionPrompt) -> PermissionChoice {
        PermissionChoice::TransportInterrupted(
            nola_host_protocol::PermissionTransportInterruptionReason::Unavailable,
        )
    }
    fn request_user_question(&self, _: &UserQuestionPrompt) -> UserQuestionAnswer {
        UserQuestionAnswer::Unavailable
    }
}

fn current_port() -> std::sync::Arc<dyn RuntimeInteraction> {
    crate::runtime_control::current_execution_control()
        .map(|control| control.interaction())
        .unwrap_or_else(|| std::sync::Arc::new(UnavailableInteraction))
}
pub(crate) fn permission_interaction_available() -> bool {
    current_port().permission_available()
}
pub(crate) fn prompt_permission(prompt: &PermissionPrompt) -> PermissionChoice {
    current_port().request_permission(prompt)
}

/// A data summary, not ANSI framing, terminal width inspection or a prompt.
pub(crate) fn permission_summary(prompt: &PermissionPrompt) -> String {
    let mut rows = vec![prompt.title.clone()];
    rows.extend(
        prompt
            .rows
            .iter()
            .map(|(label, value)| format!("{label}: {value}")),
    );
    rows.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_control::RuntimeExecutionControl;
    fn prompt() -> PermissionPrompt {
        PermissionPrompt {
            allow_conversation: true,
            kind: PermissionPromptKind::Tool,
            title: "Read owner data".into(),
            rows: vec![],
            session_label: "Session".into(),
            session_value: "read".into(),
            custom_label: "Explain".into(),
            supports_instruction: true,
        }
    }
    #[test]
    fn unavailable_is_neither_a_denial_nor_an_approval() {
        let port = UnavailableInteraction;
        assert!(!port.permission_available());
        assert_eq!(
            port.request_permission(&prompt()),
            PermissionChoice::TransportInterrupted(
                nola_host_protocol::PermissionTransportInterruptionReason::Unavailable
            )
        );
        assert_eq!(
            port.request_user_question(&UserQuestionPrompt {
                header: "Question".into(),
                question: "Choose".into(),
                options: vec![],
                allow_freeform: true
            }),
            UserQuestionAnswer::Unavailable
        );
    }
    #[derive(Debug)]
    struct ChoicePort;
    impl RuntimeInteraction for ChoicePort {
        fn permission_available(&self) -> bool {
            true
        }
        fn request_permission(&self, _: &PermissionPrompt) -> PermissionChoice {
            PermissionChoice::Once
        }
        fn request_user_question(&self, _: &UserQuestionPrompt) -> UserQuestionAnswer {
            UserQuestionAnswer::Declined
        }
    }
    #[test]
    fn explicit_child_capability_is_not_an_ambient_worker_default() {
        let owner =
            RuntimeExecutionControl::default().with_interaction(std::sync::Arc::new(ChoicePort));
        let child = owner.child();
        let _owner = owner.enter();
        std::thread::spawn(move || {
            assert!(!permission_interaction_available());
            let _child = child.enter();
            assert_eq!(prompt_permission(&prompt()), PermissionChoice::Once);
        })
        .join()
        .unwrap();
    }
    #[test]
    fn dropping_old_scope_cannot_restore_its_interaction_capability() {
        let old =
            RuntimeExecutionControl::default().with_interaction(std::sync::Arc::new(ChoicePort));
        let active = RuntimeExecutionControl::default();
        let old_scope = old.enter();
        let active_scope = active.enter();
        drop(old_scope);
        assert!(!permission_interaction_available());
        drop(active_scope);
        assert!(!permission_interaction_available());
        assert!(crate::runtime_control::current_execution_control().is_none());
        assert!(crate::execution_context::current_execution_id().is_none());
    }
}
