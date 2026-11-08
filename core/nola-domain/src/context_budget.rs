// SPDX-License-Identifier: Apache-2.0
//! Platform-independent context configuration inputs. Policy lives in Kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextBudgetFacts {
    pub context_window_tokens: usize,
    pub reserved_output_tokens: usize,
    pub prompt_token_limit: Option<usize>,
    pub auto_compact_tokens: Option<usize>,
    pub compaction_target_tokens: Option<usize>,
    pub auto_compact_percent: u8,
    pub compaction_target_percent: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderContextBudgetUpdate {
    pub config: ContextBudgetFacts,
    pub used_tokens: usize,
}

impl Default for ContextBudgetFacts {
    fn default() -> Self {
        Self {
            context_window_tokens: 128_000,
            reserved_output_tokens: 8192,
            prompt_token_limit: None,
            auto_compact_tokens: None,
            compaction_target_tokens: None,
            auto_compact_percent: 80,
            compaction_target_percent: 45,
        }
    }
}
