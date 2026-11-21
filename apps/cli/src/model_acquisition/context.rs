// SPDX-License-Identifier: Apache-2.0
use super::{MIB, TARGET_CONTEXT};
use serde_json::Value;

const WINDOW_STEPS: [u64; 10] = [
    TARGET_CONTEXT,
    98_304,
    65_536,
    32_768,
    16_384,
    8_192,
    4_096,
    2_048,
    1_024,
    512,
];

pub(super) struct Architecture {
    pub native: Option<u64>,
    pub kv: Option<u64>,
    fixed_cache: u64,
}
impl Architecture {
    pub fn detect(config: &Value, reported: Option<u64>) -> Self {
        let c = config
            .get("text_config")
            .filter(|v| v.is_object())
            .unwrap_or(config);
        let u = |k: &str| c[k].as_u64().filter(|&n| n > 0);
        let declared = u("max_position_embeddings").or_else(|| u("n_positions"));
        let native = reported
            .filter(|&n| n > 0)
            .into_iter()
            .chain(declared)
            .min();
        let estimate = cache_layout(c);
        let (kv, fixed_cache) = estimate.map_or((None, 0), |(kv, fixed)| (Some(kv), fixed));
        Self {
            native,
            kv,
            fixed_cache,
        }
    }
    pub fn windows(&self) -> impl Iterator<Item = u64> {
        let max = match (self.native, self.kv) {
            (Some(native), Some(_)) => native.min(TARGET_CONTEXT),
            (Some(native), None) => native.min(8192),
            (None, _) => 2048,
        };
        WINDOW_STEPS.into_iter().map(move |n| n.min(max))
    }
    pub fn bytes_per_token(&self) -> u64 {
        self.kv.unwrap_or(256 * 1024)
    }
    pub fn static_bytes(&self) -> u64 {
        self.fixed_cache.saturating_add(512 * MIB)
    }
    pub fn weight_limit(&self, memory: u64, window: u64) -> u64 {
        memory
            .saturating_sub(self.static_bytes())
            .saturating_sub(window.saturating_mul(self.bytes_per_token()))
            / 11
            * 10
    }
    pub fn reason(&self, context: u64) -> &'static str {
        if context >= TARGET_CONTEXT {
            "target_met"
        } else if self.native.is_none() || self.kv.is_none() {
            "metadata_unknown"
        } else if self.native == Some(context) {
            "native_limit"
        } else {
            "memory_budget"
        }
    }
}
fn product(values: &[u64]) -> Option<u64> {
    values.iter().try_fold(1u64, |p, v| p.checked_mul(*v))
}
fn cache_layout(cfg: &Value) -> Option<(u64, u64)> {
    let u = |k: &str| cfg[k].as_u64().filter(|&n| n > 0);
    // MLA has a different latent cache. Without an engine-specific layout it
    // cannot establish a verified long-window estimate from GQA dimensions.
    if cfg.get("kv_lora_rank").is_some_and(|v| !v.is_null()) {
        return None;
    }
    let layers = u("num_hidden_layers")?;
    let heads = u("num_attention_heads");
    let kv_heads = u("num_key_value_heads")
        .or(heads)?
        .max(u("num_global_key_value_heads").unwrap_or(0));
    let head = u("head_dim")
        .or_else(|| {
            let hidden = u("hidden_size")?;
            let heads = heads?;
            (hidden % heads == 0).then_some(hidden / heads)
        })?
        .max(u("global_head_dim").unwrap_or(0));
    let dim_k = u("key_head_dim").unwrap_or(head);
    let dim_v = u("value_head_dim").unwrap_or(head);
    let mut attention = layers;
    let mut fixed = 0;
    if let Some(types) = cfg.get("layer_types").filter(|v| !v.is_null()) {
        let types = types.as_array()?;
        if types.len() as u64 != layers {
            return None;
        }
        let mut recurrent = 0;
        for typ in types {
            match typ.as_str()? {
                "full_attention" | "sliding_attention" | "sliding_window" => {}
                "linear_attention" => recurrent += 1,
                // LFM2's short convolution cache is smaller than a full-length
                // attention cache. Budget a full layer as a conservative bound.
                "conv"
                    if cfg["model_type"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("lfm2")) => {}
                _ => return None,
            }
        }
        if recurrent > 0 {
            let model = cfg["model_type"].as_str()?;
            if !(model.starts_with("qwen3_5") || model.starts_with("qwen3_next")) {
                return None;
            }
            let nk = u("linear_num_key_heads")?;
            let nv = u("linear_num_value_heads")?;
            let dk = u("linear_key_head_dim")?;
            let dv = u("linear_value_head_dim")?;
            let conv = product(&[2, nk, dk])?
                .checked_add(nv.checked_mul(dv)?)?
                .checked_mul(u("linear_conv_kernel_dim")?)?;
            // GatedDeltaNet keeps a convolution ring plus one recurrent matrix;
            // budget FP32 for both, including the number of recurrent layers.
            fixed = product(&[recurrent, 4, conv.checked_add(product(&[nv, dk, dv])?)?])?;
            attention = layers.checked_sub(recurrent)?;
        }
    }
    if attention == 0 {
        return None;
    }
    // Default inference caches use half precision. Keep a conservative FP32
    // upper bound when the checkpoint explicitly requests single precision.
    let width = if cfg
        .get("dtype")
        .or(cfg.get("torch_dtype"))
        .and_then(Value::as_str)
        == Some("float32")
    {
        4
    } else {
        2
    };
    let kv = product(&[attention, kv_heads, dim_k.checked_add(dim_v)?, width])?;
    Some((kv, fixed))
}
