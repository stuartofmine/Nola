// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines surface identity and Host-specific workflow overlays.
// - Boundary: Product-wide policy belongs to Core; this module contains no tool routing or authorization.

pub(crate) struct HostPromptSet {
    pub identity: &'static str,
    pub execution_policy: Option<&'static str>,
}

use nola_core::agent::{
    SopCompletionContract, SopDefinition, SopExecutionMode, SopRetryPolicy, SopStep, SopStepBudget,
    SopVerificationContract, SOP_SCHEMA_VERSION,
};
use serde_json::json;

const PRODUCT_HOST_IDENTITY: &str = "You are Nola, developed by Gao. The active user-facing surface is an authenticated Nola product client. Present one coherent assistant experience; terminal processes, transports, and Host implementations are capabilities, not product identity.";

const MAC_IDENTITY: &str = "You are Nola, developed by Gao, inside the native Nola for Mac application. Present yourself only as Nola; local tools and Agent Host transport are implementation capabilities, not product identity.";

const CLI_IDENTITY: &str =
    "You are Nola CLI, developed by Gao: a local-first terminal coding assistant.";

const MAC_EXECUTION_POLICY: &str = "The native client renders structured activity and permission requests separately from assistant Markdown. Use advertised native browser or computer capabilities when their contracts fit the requested interaction.";

const CLI_EXECUTION_POLICY: &str = "Assistant Markdown is displayed in a terminal. Structured activity and permission requests are rendered by the interactive layer. Slash commands belong to that layer, not shell execution; use callable tool schemas for operations.";

pub(crate) fn current_host_sops() -> Vec<SopDefinition> {
    let surface = std::env::var("NOLA_CLIENT_SURFACE")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if matches!(
        surface.as_str(),
        "mac" | "macos" | "web" | "ios" | "android" | "agent_host"
    ) {
        vec![product_work_sop()]
    } else {
        vec![cli_terminal_work_sop()]
    }
}

fn product_work_sop() -> SopDefinition {
    fn step(id: &str, title: &str, depends_on: &[&str], completion: &str) -> SopStep {
        SopStep {
            id: id.to_string(),
            title: title.to_string(),
            depends_on: depends_on.iter().map(|value| value.to_string()).collect(),
            mode: SopExecutionMode::Inline,
            concurrent: id == "inspect",
            inputs: json!({"type": "object"}),
            outputs: json!({"type": "object"}),
            preconditions: Vec::new(),
            completion: SopCompletionContract {
                description: completion.to_string(),
                evidence_required: matches!(id, "inspect" | "verify" | "report"),
            },
            required_capabilities: Vec::new(),
            required_permissions: Vec::new(),
            budget: SopStepBudget::default(),
            retry: SopRetryPolicy::default(),
            artifacts: Vec::new(),
            verifier: None,
            resume_key: Some(format!("product-work:{id}")),
        }
    }
    let definition = SopDefinition {
        schema_version: SOP_SCHEMA_VERSION,
        id: "nola.product.work".to_string(),
        version: 1,
        title: "Product work".to_string(),
        inputs: json!({
            "type": "object",
            "required": ["request"],
            "properties": {
                "request": {"type": "string"},
                "workspace": {"type": "string"}
            }
        }),
        steps: vec![
            step("resolve", "Resolve request", &[], "Intent, success criteria, and relevant product context are known."),
            step("inspect", "Gather evidence", &["resolve"], "The smallest authoritative evidence needed for the request has been collected."),
            step("plan", "Plan bounded work", &["inspect"], "The execution sequence, side effects, and required permissions are explicit."),
            step("execute", "Execute work", &["plan"], "The requested work has reached a concrete outcome without undeclared side effects."),
            step("verify", "Verify outcome", &["execute"], "Observed evidence supports the claimed outcome."),
            step("report", "Report result", &["verify"], "The user receives the result, artifacts, evidence, and remaining limitations."),
        ],
        verification: SopVerificationContract::default(),
        compensation: Default::default(),
    };
    debug_assert!(definition.validate().is_ok());
    definition
}

fn cli_terminal_work_sop() -> SopDefinition {
    fn step(id: &str, title: &str, depends_on: &[&str], completion: &str) -> SopStep {
        SopStep {
            id: id.to_string(),
            title: title.to_string(),
            depends_on: depends_on.iter().map(|value| value.to_string()).collect(),
            mode: SopExecutionMode::Inline,
            concurrent: id == "inspect",
            inputs: json!({"type": "object"}),
            outputs: json!({"type": "object"}),
            preconditions: Vec::new(),
            completion: SopCompletionContract {
                description: completion.to_string(),
                evidence_required: matches!(id, "inspect" | "checkpoint" | "verify" | "report"),
            },
            required_capabilities: Vec::new(),
            required_permissions: Vec::new(),
            budget: SopStepBudget::default(),
            retry: SopRetryPolicy::default(),
            artifacts: Vec::new(),
            verifier: None,
            resume_key: Some(format!("terminal-work:{id}")),
        }
    }
    let definition = SopDefinition {
        schema_version: SOP_SCHEMA_VERSION,
        id: "cli.terminal.work".to_string(),
        version: 3,
        title: "Terminal work".to_string(),
        inputs: json!({
            "type": "object",
            "required": ["request", "workspace"],
            "properties": {
                "request": {"type": "string"},
                "workspace": {"type": "string"}
            }
        }),
        steps: vec![
            step("resolve", "Resolve request", &[], "Intent, success criteria, workspace, and repository instructions are known."),
            step("inspect", "Inspect relevant behavior", &["resolve"], "The smallest authoritative behavior path needed for the request has been inspected."),
            step("plan", "Plan bounded work", &["inspect"], "The adopted workflow has a bounded execution plan within the requested scope."),
            step("execute", "Execute changes", &["plan"], "Requested changes or operations have reached a concrete outcome within the authorized scope."),
            step("checkpoint", "Persist continuation", &["execute"], "Continuation state, when needed, preserves observed artifacts, side effects, and remaining work."),
            step("verify", "Verify outcome", &["checkpoint"], "Observed verification evidence matches the request and no verification claim exceeds what ran."),
            step("report", "Report evidence", &["verify"], "The user receives the outcome, exact artifacts, observed evidence, and remaining limitations."),
        ],
        verification: SopVerificationContract::default(),
        compensation: Default::default(),
    };
    debug_assert!(definition.validate().is_ok());
    definition
}

pub(crate) fn current_host_prompt_set() -> HostPromptSet {
    let surface = std::env::var("NOLA_CLIENT_SURFACE")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let desktop_host = std::env::var("NOLA_DESKTOP_HOST")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if matches!(surface.as_str(), "mac" | "macos")
        || (surface.is_empty() && matches!(desktop_host.as_str(), "1" | "true" | "yes"))
    {
        HostPromptSet {
            identity: MAC_IDENTITY,
            execution_policy: Some(MAC_EXECUTION_POLICY),
        }
    } else if matches!(surface.as_str(), "web" | "ios" | "android" | "agent_host") {
        HostPromptSet {
            identity: PRODUCT_HOST_IDENTITY,
            execution_policy: None,
        }
    } else {
        HostPromptSet {
            identity: CLI_IDENTITY,
            execution_policy: Some(CLI_EXECUTION_POLICY),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_overlay_contains_only_terminal_workflow() {
        assert!(CLI_EXECUTION_POLICY.contains("displayed in a terminal"));
        assert!(CLI_EXECUTION_POLICY.contains("Slash commands"));
        assert!(!CLI_EXECUTION_POLICY.contains("1."));
        assert_eq!(cli_terminal_work_sop().steps.len(), 7);
        for duplicated_core_policy in [
            "web_search", "memory_save", "grant covers", "current date",
            "foreground", "explicit authorization",
        ] {
            assert!(!CLI_EXECUTION_POLICY.contains(duplicated_core_policy));
        }
    }

    #[test]
    fn surface_overlays_do_not_redefine_task_scope_or_require_sop_activation() {
        for overlay in [CLI_EXECUTION_POLICY, MAC_EXECUTION_POLICY] {
            for obsolete_policy in [
                "activate", "SOP", "scratch", "install dependencies",
                "unless the user requested", "before performing",
            ] {
                assert!(!overlay.contains(obsolete_policy), "{obsolete_policy}");
            }
        }
    }

    #[test]
    fn cli_workflow_is_a_valid_versioned_sop_dag() {
        let definition = cli_terminal_work_sop();
        definition.validate().unwrap();
        assert_eq!(definition.schema_version, SOP_SCHEMA_VERSION);
        assert_eq!(definition.version, 3);
        assert_eq!(
            definition
                .steps
                .iter()
                .find(|step| step.id == "verify")
                .unwrap()
                .depends_on,
            vec!["checkpoint"]
        );
        assert_eq!(definition.steps.last().unwrap().depends_on, vec!["verify"]);
        assert!(definition
            .steps
            .iter()
            .all(|step| step.resume_key.is_some()));
    }

    #[test]
    fn product_workflow_is_a_valid_versioned_sop_dag() {
        let definition = product_work_sop();
        definition.validate().unwrap();
        assert_eq!(definition.schema_version, SOP_SCHEMA_VERSION);
        assert_eq!(definition.id, "nola.product.work");
        assert_eq!(definition.steps.last().unwrap().id, "report");
        assert!(definition
            .steps
            .iter()
            .all(|step| step.resume_key.is_some()));
    }

    #[test]
    fn native_identity_never_exposes_cli_as_product_identity() {
        assert!(MAC_IDENTITY.contains("native Nola for Mac"));
        assert!(!MAC_IDENTITY.contains("Nola CLI"));
        assert!(!PRODUCT_HOST_IDENTITY.contains("Nola CLI"));
    }
}
