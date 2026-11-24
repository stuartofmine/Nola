// SPDX-License-Identifier: Apache-2.0
//! Prefix-stable tool catalog.
//!
//! KV-cache invariant:
//! Turn 0 exposes ONLY `ToolTier::Core` (~12 orthogonal primitives) in deterministic
//! registration order. Deferred tools activated via `tool_search` are strictly APPENDED
//! to the tail and NEVER reorder or remove earlier tool schemas.

use std::collections::BTreeSet;

use nola_tools::ToolRegistry;
use nola_types::{HostCapabilities, ToolSchema, ToolTier};

/// Controls which deferred tools are exposed upfront on Turn 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForceToolSet {
    /// Turn 0 exposes only `ToolTier::Core` (plus `InteractiveOnly` when host supports it).
    #[default]
    Auto,
    /// Turn 0 exposes Core + write/mutating Deferred tools.
    Write,
    /// Turn 0 exposes Core + all Deferred tools.
    All,
}

/// Tracks the ordered set of tool schemas currently visible to the model.
#[derive(Debug, Clone)]
pub struct ToolCatalog {
    active_names: Vec<String>,
    active_set: BTreeSet<String>,
    host_capabilities: HostCapabilities,
}

impl ToolCatalog {
    pub fn build_initial(
        registry: &ToolRegistry,
        caps: HostCapabilities,
        force_tool_set: ForceToolSet,
    ) -> Self {
        let mut active_names = Vec::new();
        let mut active_set = BTreeSet::new();
        for spec in registry.all_specs() {
            let include = match spec.tier {
                ToolTier::Core => true,
                ToolTier::Deferred => match force_tool_set {
                    ForceToolSet::Auto => false,
                    ForceToolSet::Write => !spec.read_only,
                    ForceToolSet::All => true,
                },
                ToolTier::InteractiveOnly => caps.interactive_user_input,
            };
            if include && active_set.insert(spec.schema.name.clone()) {
                active_names.push(spec.schema.name);
            }
        }
        Self {
            active_names,
            active_set,
            host_capabilities: caps,
        }
    }

    fn allows_tool(&self, name: &str, registry: &ToolRegistry) -> bool {
        let Some(handler) = registry.get(name) else {
            return false;
        };
        let spec = handler.spec();
        if spec.tier == ToolTier::InteractiveOnly && !self.host_capabilities.interactive_user_input {
            return false;
        }
        true
    }

    /// Restore previously activated tool names (preserving exact prior prefix order on `--resume`).
    pub fn restore_activated(&mut self, names: &[String], registry: &ToolRegistry) {
        for name in names {
            if self.allows_tool(name, registry) && self.active_set.insert(name.clone()) {
                self.active_names.push(name.clone());
            }
        }
    }

    /// Append newly discovered deferred tools strictly at the tail (0 prefix drift!).
    pub fn activate_tail(&mut self, names: &[String], registry: &ToolRegistry) -> Vec<String> {
        let mut added = Vec::new();
        for name in names {
            if self.allows_tool(name, registry) && self.active_set.insert(name.clone()) {
                self.active_names.push(name.clone());
                added.push(name.clone());
            }
        }
        added
    }

    /// The ONLY authorized way to compile tool schemas for `StructuredModelRequest`.
    pub fn compile_preserving_prefix_order(&self, registry: &ToolRegistry) -> Vec<ToolSchema> {
        self.active_names
            .iter()
            .filter_map(|name| registry.get(name).map(|h| h.spec().schema))
            .collect()
    }

    pub fn active_names(&self) -> &[String] {
        &self.active_names
    }
}
