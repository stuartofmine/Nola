// SPDX-License-Identifier: Apache-2.0
//! Immutable tool invocation policy, not an authorization receipt.
//!
//! Configuration composition owns construction. Application/Core still admit
//! concrete effects and own durable grants; a discovered tool or allowed root
//! never changes this snapshot. No parser, prompt, model credentials or UI state
//! belong here.
use crate::runtime_config::PermissionRule;

#[derive(Clone, Debug)]
pub struct ToolExecutionPolicy {
    bash_prefixes: Vec<String>,
    auto_approve: bool,
    network_policy: String,
    permission_rules: Vec<PermissionRule>,
    bound_file_workspace: Option<std::path::PathBuf>,
    process_sandbox: Option<crate::process_sandbox::FrozenProcessSandbox>,
}

impl ToolExecutionPolicy {
    pub fn new(
        bash_prefixes: Vec<String>,
        auto_approve: bool,
        network_policy: String,
        permission_rules: Vec<PermissionRule>,
    ) -> Self {
        Self { bash_prefixes, auto_approve, network_policy, permission_rules, bound_file_workspace: None, process_sandbox: None }
    }

    pub(crate) fn with_process_sandbox(mut self,spec:Option<crate::process_sandbox::FrozenProcessSandbox>)->Self {self.process_sandbox=spec;self}
    pub(crate) fn process_sandbox(&self)->Option<&crate::process_sandbox::FrozenProcessSandbox> {self.process_sandbox.as_ref()}
    pub(crate) fn with_bound_file_workspace(mut self, root:Option<std::path::PathBuf>)->Self {
        self.bound_file_workspace=root;self
    }
    pub(crate) fn bound_file_workspace(&self)->Option<&std::path::Path> {self.bound_file_workspace.as_deref()}
    pub fn bash_prefixes(&self) -> &[String] { &self.bash_prefixes }
    pub fn auto_approve(&self) -> bool { self.auto_approve }
    pub fn network_policy(&self) -> &str { &self.network_policy }
    pub fn permission_rules(&self) -> &[PermissionRule] { &self.permission_rules }

    /// An effect admitted through Core must not inherit blanket auto approval
    /// in its local tool adapter. Retain every other restriction unchanged.
    pub fn without_auto_approval(&self) -> Self {
        Self { auto_approve: false, ..self.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_snapshot_and_effect_narrowing_do_not_mutate_parent() {
        let mut prefixes = vec!["git status".into()];
        let mut rules = vec![PermissionRule {
            permission: "write".into(), pattern: "*".into(),
            action: crate::runtime_config::PermissionAction::Deny,
        }];
        let policy = ToolExecutionPolicy::new(prefixes.clone(), true, "ask".into(), rules.clone());
        prefixes.push("arbitrary new command".into());
        rules.clear();
        let effect_policy = policy.without_auto_approval();
        assert!(policy.auto_approve());
        assert!(!effect_policy.auto_approve());
        assert_eq!(policy.bash_prefixes(), &["git status"]);
        assert_eq!(effect_policy.bash_prefixes(), policy.bash_prefixes());
        assert_eq!(effect_policy.network_policy(), "ask");
        assert_eq!(effect_policy.permission_rules(), policy.permission_rules());
        assert_eq!(effect_policy.permission_rules()[0].action,
            crate::runtime_config::PermissionAction::Deny);
    }
}
