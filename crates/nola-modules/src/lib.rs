// SPDX-License-Identifier: Apache-2.0
//! All Nola v2 business modules implemented once cleanly on top of `nola-store`
//! (`module_records` + FTS5 `memory_items`) and exposed both as Rust services
//! (for CLI/Web/Daemon) and as `ToolTier::Deferred` tools in `ToolRegistry`.
//!
//! Retained business modules:
//! - Skills (`SKILL.md` discovery + tail-injected `skill_load`)
//! - Memory & Code-Memory (FTS5 long-term memory + repo index)
//! - Goals & SOPs
//! - AgentRoom, ExpertPeer & Subagents
//! - DataSources
//! - Automations & MessagingChannels
//! - LocalModels / Acquisition & Plugins

pub mod automations_messaging;
pub mod data_sources;
pub mod goals_sops;
pub mod local_models_plugins;
pub mod memory;
pub mod rooms_peers;
pub mod skills;

use std::path::PathBuf;
use std::sync::Arc;

use nola_store::{Store, StoreError};
use nola_tools::ToolRegistry;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Unified facade exposing every retained Nola business module over a single `Store`.
#[derive(Clone)]
pub struct ModulesSuite {
    pub store: Store,
    pub workspace_root: PathBuf,
    pub user_home: Option<PathBuf>,
}

impl ModulesSuite {
    pub fn new(store: Store, workspace_root: PathBuf, user_home: Option<PathBuf>) -> Self {
        Self {
            store,
            workspace_root,
            user_home,
        }
    }

    /// Generic typed CRUD helper shared across all business modules (eliminates 10x boilerplate!).
    pub fn upsert_record<T: Serialize>(&self, kind: &str, id: &str, rec: &T) -> Result<(), StoreError> {
        self.store.put_module_record(kind, id, rec)
    }

    pub fn get_record<T: DeserializeOwned>(
        &self,
        kind: &str,
        id: &str,
    ) -> Result<Option<T>, StoreError> {
        self.store.get_module_record(kind, id)
    }

    pub fn list_records<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>, StoreError> {
        self.store.list_module_records(kind)
    }

    pub fn delete_record(&self, kind: &str, id: &str) -> Result<bool, StoreError> {
        self.store.delete_module_record(kind, id)
    }

    /// Register all business-module tools as `ToolTier::Deferred` on `ToolRegistry`
    /// so Turn-0 tool schemas stay compact (~12 core tools) while all modules remain callable!
    pub fn register_tools(self: &Arc<Self>, registry: &mut ToolRegistry) {
        skills::register_tools(self, registry);
        memory::register_tools(self, registry);
        goals_sops::register_tools(self, registry);
        rooms_peers::register_tools(self, registry);
        data_sources::register_tools(self, registry);
        automations_messaging::register_tools(self, registry);
        local_models_plugins::register_tools(self, registry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_tools::ToolContext;
    use serde_json::json;

    #[test]
    fn all_business_modules_register_and_operate_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join(".nola/skills/demo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo\ndescription: Demo workflow\n---\nAlways run tests first.\n",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let suite = Arc::new(ModulesSuite::new(store, dir.path().to_path_buf(), None));
        let mut reg = ToolRegistry::with_builtin_tools();
        suite.register_tools(&mut reg);
        let ctx = ToolContext::new(dir.path());

        // 1. Skills
        let sk = reg.execute("skill_load", &ctx, json!({"name": "demo"}));
        assert!(!sk.is_error);
        assert!(sk.payload.to_model_value()["body"]
            .as_str()
            .unwrap()
            .contains("Always run tests first"));

        // 2. Memory (FTS5)
        let ms = reg.execute(
            "memory_save",
            &ctx,
            json!({"title": "Deploy", "content": "Use blue-green rollout", "tags": ["ops"]}),
        );
        assert!(!ms.is_error);
        let mq = reg.execute("memory_search", &ctx, json!({"query": "blue-green"}));
        assert_eq!(mq.payload.to_model_value()["count"], 1);

        // 3. Goals & SOPs
        let g = reg.execute(
            "goal_manage",
            &ctx,
            json!({"action": "create", "title": "Release v2", "objective": "Pass all gates"}),
        );
        assert!(!g.is_error);
        let sop = reg.execute(
            "sop_manage",
            &ctx,
            json!({"action": "create", "name": "PR Review", "steps": ["check diff", "run tests"]}),
        );
        assert!(!sop.is_error);

        // 4. ExpertPeer & AgentRoom
        let peer = reg.execute(
            "room_manage",
            &ctx,
            json!({"action": "create_expert", "name": "Rust Auditor", "role": "Security", "system_prompt": "Audit rust"}),
        );
        assert!(!peer.is_error);
        let room = reg.execute(
            "room_manage",
            &ctx,
            json!({"action": "create_room", "name": "Architecture"}),
        );
        assert!(!room.is_error);

        // 5. DataSources
        let ds = reg.execute(
            "data_source_manage",
            &ctx,
            json!({"action": "create", "name": "Local Docs", "kind": "dir", "location": "."}),
        );
        assert!(!ds.is_error);

        // 6. Automations & Messaging
        let auto = reg.execute(
            "automation_manage",
            &ctx,
            json!({"action": "create", "name": "Nightly", "schedule": "0 2 * * *", "prompt": "Check CI"}),
        );
        assert!(!auto.is_error);
        let ch = reg.execute(
            "messaging_manage",
            &ctx,
            json!({"action": "create", "name": "Ops Hook", "kind": "webhook", "endpoint": "https://example.invalid/hook"}),
        );
        assert!(!ch.is_error);

        // 7. LocalModels & Plugins
        let lm = reg.execute(
            "local_model_manage",
            &ctx,
            json!({"action": "register", "name": "qwen3-8b", "engine": "strata"}),
        );
        assert!(!lm.is_error);
        let pl = reg.execute(
            "plugin_manage",
            &ctx,
            json!({"action": "register", "name": "git-helper", "version": "1.0.0", "entry_path": "./plugin.js"}),
        );
        assert!(!pl.is_error);
    }
}
