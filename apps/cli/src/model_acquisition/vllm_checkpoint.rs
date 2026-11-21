// SPDX-License-Identifier: Apache-2.0
//! Directory formats supported by vllm-metal's stable MLX runner on Apple silicon.
//! https://github.com/vllm-project/vllm-metal/blob/v0.33.0/docs/supported_models.md
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Backend {
    Standard,
    MetalMlx,
    MetalMps,
}
impl Backend {
    pub(super) fn current() -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            if !super::engine_setup::metal_supported() {
                return Err("vLLM Metal requires macOS 15 or later on Apple silicon".into());
            }
            return match std::env::var("VLLM_METAL_BACKEND").as_deref() {
                Ok("mps") => Ok(Self::MetalMps),
                Ok("mlx" | "") | Err(_) => Ok(Self::MetalMlx),
                Ok(_) => Err("Unsupported VLLM_METAL_BACKEND: select mlx or mps".into()),
            };
        }
        #[cfg(not(target_os = "macos"))]
        Ok(Self::Standard)
    }

    pub(super) fn accepts_repo(self, mlx: bool, gguf: bool) -> bool {
        !gguf && (!mlx || self == Self::MetalMlx)
    }

    pub(super) fn checkpoint(self, cfg: &Value) -> Result<Option<String>, String> {
        if !cfg.is_object() {
            return Err("Invalid vLLM model configuration".into());
        }
        let mlx = cfg
            .get("quantization")
            .filter(|v| v.is_object() && !v.as_object().is_some_and(|v| v.is_empty()));
        let q = cfg
            .get("quantization_config")
            .filter(|v| v.is_object() && !v.as_object().is_some_and(|v| v.is_empty()));
        let method = q
            .and_then(|v| v["quant_method"].as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let native = (mlx.is_some() && matches!(method.as_str(), "" | "mlx"))
            || method == "mlx"
            || (method.is_empty()
                && q.is_some_and(|v| {
                    v["mode"]
                        .as_str()
                        .is_some_and(|m| matches!(m, "affine" | "mxfp4" | "mxfp8" | "nvfp4"))
                }));
        if self == Self::Standard {
            return if native {
                Err("MLX-quantized weights require the Apple silicon MLX backend. Select a standard vLLM safetensors checkpoint.".into())
            } else {
                Ok(None)
            };
        }
        let arch = cfg["model_type"].as_str().unwrap_or("");
        if matches!(arch, "lfm3" | "lfm3_vl" | "minicpm3" | "minicpm4")
            && cfg.get("auto_map").is_some_and(Value::is_object)
        {
            return Err("This checkpoint requires custom repository code; select a model supported by the local vLLM Metal runner.".into());
        }
        if matches!(arch, "ministral3" | "devstral_small_2") {
            return Err(
                "This architecture is not supported by the stable vLLM Metal runner.".into(),
            );
        }
        if self == Self::MetalMps && (mlx.is_some() || q.is_some()) {
            return Err("The Metal MPS runner needs unquantized safetensors. Select the MLX runner for MLX quantization.".into());
        }
        if method == "awq" {
            let v = q.unwrap();
            let bits = v.get("bits").or(v.get("w_bit")).and_then(Value::as_u64);
            let group = v
                .get("group_size")
                .or(v.get("q_group_size"))
                .and_then(Value::as_u64);
            if !matches!(arch, "qwen2" | "llama" | "mistral")
                || bits != Some(4)
                || group != Some(128)
                || v.get("zero_point")
                    .is_some_and(|value| value.as_bool() != Some(true))
                || v.get("version").is_some_and(|value| {
                    !value
                        .as_str()
                        .is_some_and(|s| s.eq_ignore_ascii_case("gemm"))
                })
            {
                return Err("vLLM Metal supports verified Qwen2.5, Llama and Mistral AWQ checkpoints with 4-bit GEMM, group size 128 and zero points.".into());
            }
            return Ok(Some("AWQ 4-bit GEMM".into()));
        }
        if native {
            let v = mlx.or(q).ok_or("Missing MLX quantization settings")?;
            let mode = v["mode"].as_str().unwrap_or("affine");
            let bits = v["bits"].as_u64();
            let group = v["group_size"].as_u64();
            let supported = match mode {
                "affine" => {
                    matches!(bits, Some(1 | 2 | 3 | 4 | 5 | 6 | 8))
                        && matches!(group, Some(32 | 64 | 128))
                }
                "mxfp4" => (bits, group) == (Some(4), Some(32)),
                "mxfp8" => (bits, group) == (Some(8), Some(32)),
                "nvfp4" => (bits, group) == (Some(4), Some(16)),
                _ => false,
            };
            if !supported {
                return Err(
                    "Unsupported MLX quantization mode, bits or group size for vLLM Metal".into(),
                );
            }
            return Ok(Some(format!("MLX {}-bit {mode}", bits.unwrap())));
        }
        if q.is_some() || mlx.is_some() {
            return Err(format!("Unsupported directory quantization for vLLM Metal: {}. Select unquantized safetensors, MLX or a compatible AWQ checkpoint.",if method.is_empty(){"unknown"}else{&method}));
        }
        if let Some(dtype) = cfg
            .get("dtype")
            .or(cfg.get("torch_dtype"))
            .and_then(Value::as_str)
        {
            if !matches!(
                dtype,
                "float16" | "half" | "bfloat16" | "float32" | "float" | "auto"
            ) {
                return Err(format!(
                    "Unsupported unquantized vLLM Metal data type: {dtype}"
                ));
            }
        }
        Ok(None)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn mlx_directories_follow_the_runtime_and_actual_weight_settings() {
        let cfg = json!({"model_type":"qwen2","quantization":{"group_size":64,"bits":4}});
        assert!(Backend::MetalMlx.accepts_repo(true, false));
        assert!(!Backend::Standard.accepts_repo(true, false));
        assert!(!Backend::MetalMps.accepts_repo(true, false));
        assert!(!Backend::MetalMlx.accepts_repo(false, true));
        assert_eq!(
            Backend::MetalMlx.checkpoint(&cfg).unwrap().as_deref(),
            Some("MLX 4-bit affine")
        );
        assert!(Backend::Standard.checkpoint(&cfg).is_err());
        assert!(Backend::MetalMps.checkpoint(&cfg).is_err());
        assert!(Backend::MetalMlx
            .checkpoint(&json!({"quantization":{"bits":7,"group_size":64}}))
            .is_err());
        assert!(Backend::MetalMlx
            .checkpoint(&json!({"model_type":"qwen3","torch_dtype":"bfloat16"}))
            .is_ok());
    }
    #[test]
    fn awq_cuda_formats_and_native_mlx_are_distinguished_from_the_file_contents() {
        let cfg = json!({"model_type":"qwen2","quantization_config":{"quant_method":"awq","w_bit":4,"q_group_size":128,"zero_point":true,"version":"GEMM"}});
        assert!(Backend::MetalMlx.checkpoint(&cfg).is_ok());
        let mut defaults = cfg.clone();
        defaults["quantization_config"]
            .as_object_mut()
            .unwrap()
            .remove("zero_point");
        defaults["quantization_config"]
            .as_object_mut()
            .unwrap()
            .remove("version");
        assert!(Backend::MetalMlx.checkpoint(&defaults).is_ok());
        for (key, value) in [
            ("version", json!("gemv")),
            ("zero_point", json!(false)),
            ("w_bit", json!(8)),
            ("q_group_size", json!(64)),
            ("quant_method", json!("gptq")),
        ] {
            let mut changed = cfg.clone();
            changed["quantization_config"][key] = value;
            assert!(Backend::MetalMlx.checkpoint(&changed).is_err());
            assert!(Backend::Standard.checkpoint(&changed).is_ok());
        }
    }
}
