// SPDX-License-Identifier: Apache-2.0
//! Runtime-owned Skill source selection, independent of filesystem grants.
//!
//! None selects the managed standalone catalog; an explicit empty list disables
//! Skills. Clones retain the same immutable selection across worker/permission
//! boundaries. This is neither a tool grant nor a snapshot of file contents.

use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillRuntimeContext {
    configured_dirs: Option<Arc<[PathBuf]>>,
    plugins: Option<crate::plugin_runtime::SharedPluginSession>,
}

impl SkillRuntimeContext {
    pub(crate) fn from_configured_dirs(configured_dirs: Option<Vec<PathBuf>>) -> Self {
        Self {
            configured_dirs: configured_dirs.map(Arc::from),
            plugins: None,
        }
    }

    pub(crate) fn configured_dirs(&self) -> Option<&[PathBuf]> {
        self.configured_dirs.as_deref()
    }

    pub(crate) fn with_plugins(mut self, plugins: crate::plugin_runtime::SharedPluginSession) -> Self {
        self.plugins = Some(plugins);
        self
    }

    pub(crate) fn plugin_skills(&self) -> Result<Vec<nola_plugin_host::RuntimeSkill>, String> {
        if !self.enabled() { return Ok(Vec::new()); }
        self.plugins.as_ref().map(|plugins| plugins.skills()).transpose().map(Option::unwrap_or_default)
    }

    pub(crate) fn enabled(&self) -> bool {
        self.configured_dirs
            .as_ref()
            .is_none_or(|directories| !directories.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::SkillRuntimeContext;
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn managed_disabled_and_configured_sources_remain_distinct() {
        let managed = SkillRuntimeContext::default();
        let disabled = SkillRuntimeContext::from_configured_dirs(Some(Vec::new()));
        let configured = SkillRuntimeContext::from_configured_dirs(Some(vec![PathBuf::from(
            "/fixture/runtime-a",
        )]));
        assert_eq!(managed.configured_dirs(), None);
        assert!(managed.enabled());
        assert_eq!(disabled.configured_dirs(), Some([].as_slice()));
        assert!(!disabled.enabled());
        assert_eq!(
            configured.configured_dirs(),
            Some([PathBuf::from("/fixture/runtime-a")].as_slice())
        );
        assert!(configured.enabled());
        assert_ne!(managed, disabled);
    }

    #[test]
    fn another_runtime_and_mutating_the_input_cannot_rebind_a_clone() {
        let mut input = vec![PathBuf::from("/fixture/runtime-a")];
        let original = SkillRuntimeContext::from_configured_dirs(Some(input.clone()));
        let worker = original.clone();
        input[0] = PathBuf::from("/fixture/runtime-b");
        let another = SkillRuntimeContext::from_configured_dirs(Some(input));
        assert_eq!(original, worker);
        assert_ne!(worker, another);
        assert_eq!(
            worker.configured_dirs(),
            Some([PathBuf::from("/fixture/runtime-a")].as_slice())
        );
        assert!(Arc::ptr_eq(
            original.configured_dirs.as_ref().unwrap(),
            worker.configured_dirs.as_ref().unwrap()
        ));
    }
}
