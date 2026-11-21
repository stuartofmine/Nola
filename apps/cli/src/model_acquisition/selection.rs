// SPDX-License-Identifier: Apache-2.0
use super::{Engine, FileSpec, Manifest};
use std::collections::{BTreeMap, HashSet};

pub(super) fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() < 1024
        && path.split('/').all(|part| {
            let device = part.split('.').next().unwrap_or("").to_ascii_lowercase();
            !part.is_empty()
                && part.len() <= 240
                && !part.starts_with('.')
                && !part.ends_with(['.', ' '])
                && !part
                    .chars()
                    .any(|c| c.is_control() || r#"\:*?"<>|"#.contains(c))
                && !matches!(
                    device.as_str(),
                    "con" | "prn" | "aux" | "nul" | "conin$" | "conout$"
                )
                && !(device.len() == 4
                    && (device.starts_with("com") || device.starts_with("lpt"))
                    && device.as_bytes()[3].is_ascii_digit())
        })
}

fn gguf_group(path: &str) -> Option<(String, usize, usize)> {
    let lower = path.to_ascii_lowercase();
    let stem = lower.strip_suffix(".gguf")?;
    if [
        "mmproj",
        "projector",
        "imatrix",
        "vocoder",
        "adapter",
        "big-endian",
        "mtp-",
    ]
    .iter()
    .any(|s| stem.contains(s))
    {
        return None;
    }
    if let Some((prefix, total)) = stem.rsplit_once("-of-") {
        let (name, index) = prefix.rsplit_once('-')?;
        let total: usize = total.parse().ok()?;
        let index: usize = index.parse().ok()?;
        if index == 0 || index > total || total > 99999 {
            return None;
        }
        return Some((name.to_owned(), index, total));
    }
    Some((stem.to_owned(), 1, 1))
}

fn quantization_token(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .find(|piece| {
            let rest = piece
                .strip_prefix("iq")
                .or_else(|| piece.strip_prefix("tq"))
                .or_else(|| piece.strip_prefix('q'));
            rest.and_then(|r| r.chars().next())
                .is_some_and(|c| c.is_ascii_digit())
        })
        .map(str::to_owned)
}
pub(super) fn gguf_quantized(name: &str) -> bool {
    quantization_token(name).is_some()
}

fn below_three_bits(name: &str) -> bool {
    quantization_token(name).is_some_and(|t| {
        t.trim_start_matches(['i', 't', 'q'])
            .starts_with(['1', '2'])
    })
}

fn quality_rank(name: &str) -> usize {
    let token = quantization_token(name);
    // Unsloth's dynamic XL keeps selected tensors at higher precision.
    let rank = [
        "q4_k_xl", "q4_k_m", "q4_k_s", "q4_0", "q5_k_xl", "q5_k_m", "q5_k_s", "q6_k", "q8_0",
        "iq4_xs", "iq4_nl", "q3_k_xl", "q3_k_m", "q3_k_s",
    ];
    if let Some(t) = token {
        if let Some(i) = rank.iter().position(|v| *v == t) {
            return i;
        }
        return match t.trim_start_matches(['i', 't', 'q']).chars().next() {
            Some('1') => 50,
            Some('2') => 40,
            _ => 20,
        };
    }
    if name.contains("f16") || name.contains("f32") {
        26
    } else {
        30
    }
}

fn gguf(files: &[FileSpec], budget: Option<u64>) -> Result<Vec<FileSpec>, String> {
    let mut groups = BTreeMap::<String, Vec<(usize, usize, &FileSpec)>>::new();
    for file in files {
        if let Some((name, i, n)) = gguf_group(&file.path) {
            groups.entry(name).or_default().push((i, n, file));
        }
    }

    let mut options = Vec::new();
    for (name, mut shards) in groups {
        shards.sort_by_key(|(i, _, _)| *i);
        if !shards
            .iter()
            .enumerate()
            .all(|(offset, (i, n, f))| *i == offset + 1 && *n == shards.len() && f.bytes > 0)
        {
            continue;
        }
        let bytes = shards
            .iter()
            .try_fold(0u64, |acc, (_, _, f)| acc.checked_add(f.bytes))
            .ok_or("Invalid model size")?;
        let rank = quality_rank(&name);
        // Preserve context headroom. When nothing fits, select the smallest complete variant.
        let exceeds = budget.is_some_and(|b| bytes > b);
        options.push((
            (
                exceeds,
                if exceeds { bytes } else { rank as u64 },
                bytes,
                name,
            ),
            shards,
        ));
    }
    let (_, shards) = options
        .into_iter()
        .min_by(|a, b| a.0.cmp(&b.0))
        .ok_or("No complete GGUF model in this repository")?;
    Ok(shards.into_iter().map(|(_, _, f)| f.clone()).collect())
}

/// Try the longest supportable window first. A sub-three-bit variant is never
/// selected just to cross a context threshold over a useful higher-precision build.
pub(super) fn context_gguf_budget(
    files: &[FileSpec],
    budgets: impl IntoIterator<Item = u64>,
    fallback: u64,
) -> u64 {
    for budget in budgets {
        if let Ok(selected) = gguf(files, Some(budget)) {
            if !below_three_bits(&selected[0].path)
                && selected
                    .iter()
                    .try_fold(0u64, |n, f| n.checked_add(f.bytes))
                    .is_some_and(|bytes| bytes <= budget)
            {
                return budget;
            }
        }
    }
    fallback
}
pub(super) fn primary(
    engine: Engine,
    repo: &str,
    manifest: &Manifest,
) -> Result<Vec<FileSpec>, String> {
    primary_for_budget(
        engine,
        repo,
        manifest,
        crate::local_models::physical_memory_bytes().map(|m| m / 4 * 3),
    )
}

pub(super) fn primary_for_budget(
    engine: Engine,
    repo: &str,
    manifest: &Manifest,
    budget: Option<u64>,
) -> Result<Vec<FileSpec>, String> {
    let files = &manifest.files;
    let selected = if engine == Engine::Llama {
        gguf(files, budget)?
    } else {
        if engine == Engine::Vllm
            && files
                .iter()
                .any(|f| f.path.to_ascii_lowercase().ends_with(".gguf"))
            && !files.iter().any(|f| f.path.ends_with(".safetensors"))
        {
            return Err("Choose llama.cpp for GGUF in Nola. The vLLM runtime uses a model directory with safetensors, config and tokenizer files.".into());
        }
        if !engine.matches_metadata(repo, &manifest.tags) {
            return Err(format!(
                "{repo} is not a compatible {} snapshot",
                engine.name()
            ));
        }
        let root: Vec<_> = files.iter().filter(|f| !f.path.contains('/')).collect();
        let has = |name: &str| root.iter().any(|f| f.path == name && f.bytes > 0);
        if !has("config.json") || !root.iter().any(|f| is_tokenizer(&f.path)) {
            return Err("This snapshot is missing the model config or tokenizer".into());
        }
        let indexed = has("model.safetensors.index.json");
        if !indexed && !has("model.safetensors") {
            return Err("This engine requires a complete safetensors model".into());
        }
        let mut picked: Vec<_> = files
            .iter()
            .filter(|f| {
                let p = f.path.as_str();
                p == "config.json"
                    || is_tokenizer(p)
                    || is_accessory(p)
                    || (p.starts_with("chat_templates/") && p.ends_with(".jinja"))
                    || if indexed {
                        p == "model.safetensors.index.json" || p.ends_with(".safetensors")
                    } else {
                        p == "model.safetensors"
                    }
            })
            .cloned()
            .collect();
        picked.sort_by(|a, b| a.path.cmp(&b.path));
        picked
    };
    let mut paths = HashSet::new();
    for f in &selected {
        if !safe_path(&f.path) || !paths.insert(f.path.to_lowercase()) {
            return Err("Model contains unsafe or colliding artifact paths".into());
        }
    }
    Ok(selected)
}
fn is_tokenizer(p: &str) -> bool {
    matches!(
        p,
        "tokenizer.json"
            | "tokenizer.model"
            | "sentencepiece.bpe.model"
            | "spiece.model"
            | "tekken.json"
            | "vocab.json"
            | "vocab.txt"
    ) || p.ends_with(".tiktoken")
}
fn is_accessory(p: &str) -> bool {
    matches!(
        p,
        "tokenizer_config.json"
            | "generation_config.json"
            | "special_tokens_map.json"
            | "added_tokens.json"
            | "merges.txt"
            | "processor_config.json"
            | "preprocessor_config.json"
            | "video_preprocessor_config.json"
            | "chat_template.jinja"
            | "chat_template.json"
            | "quantize_config.json"
            | "quantization_config.json"
    )
}

pub(super) fn apply_index(
    files: &mut Vec<FileSpec>,
    doc: &serde_json::Value,
) -> Result<(), String> {
    let mut required = HashSet::new();
    for target in doc["weight_map"]
        .as_object()
        .ok_or("Invalid safetensors index")?
        .values()
    {
        let path = target.as_str().ok_or("Invalid safetensors shard name")?;
        if !safe_path(path) || !path.ends_with(".safetensors") {
            return Err("Invalid safetensors shard name".into());
        }
        required.insert(path);
    }
    if required.is_empty()
        || !required
            .iter()
            .all(|n| files.iter().any(|f| f.path == **n && f.bytes > 0))
    {
        return Err("Missing safetensors shards listed by model.safetensors.index.json".into());
    }
    files.retain(|f| !f.path.ends_with(".safetensors") || required.contains(f.path.as_str()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(s: &str, size: u64) -> FileSpec {
        FileSpec {
            path: s.into(),
            bytes: size,
            revision: "a".repeat(40),
            sha256: None,
        }
    }
    #[test]
    fn quantization_groups_require_all_shards_and_fit_ram() {
        let files = vec![
            file("mmproj.gguf", 1),
            file("model-Q4_K_M-00001-of-00002.gguf", 30),
            file("model-Q4_K_M-00002-of-00002.gguf", 30),
            file("model-Q3_K_M.gguf", 15),
            file("broken-Q4_K_M-00001-of-00005.gguf", 1),
        ];
        assert_eq!(gguf(&files, Some(100)).unwrap().len(), 2);
        assert_eq!(gguf(&files, Some(24)).unwrap()[0].path, "model-Q3_K_M.gguf");
    }
    #[test]
    fn context_reservation_selects_a_complete_useful_quantization() {
        let files = [
            file("model-UD-Q4_K_XL.gguf", 65),
            file("model-Q4_K_M.gguf", 60),
            file("model-Q3_K_M.gguf", 50),
            file("model-UD-IQ1_S.gguf", 22),
        ];
        assert_eq!(
            gguf(&files, Some(70)).unwrap()[0].path,
            "model-UD-Q4_K_XL.gguf"
        );
        let budget = context_gguf_budget(&files, [30, 55, 70], 80);
        assert_eq!(
            gguf(&files, Some(budget)).unwrap()[0].path,
            "model-Q3_K_M.gguf"
        );
        let low = [
            file("model-UD-IQ1_S.gguf", 22),
            file("model-IQ2_M.gguf", 32),
        ];
        assert_eq!(gguf(&low, Some(34)).unwrap()[0].path, "model-IQ2_M.gguf");
        assert!(gguf_quantized("model-UD-IQ1_S.gguf"));
        assert!(!gguf_quantized("model-BF16.gguf"));
    }
    #[test]
    fn excludes_unsafe_paths_and_incomplete_index() {
        for p in [
            "../a", "a/../b", "AUX.json", "C:/model", "a\\b", "A.con/", "a:.json", "b.",
        ] {
            assert!(!safe_path(p), "{p}");
        }
        let mut files = vec![
            file("model-1.safetensors", 8),
            file("unrelated.safetensors", 10),
        ];
        assert!(apply_index(
            &mut files,
            &serde_json::json!({"weight_map":{"k":"absent.safetensors"}})
        )
        .is_err());
        apply_index(
            &mut files,
            &serde_json::json!({"weight_map":{"k":"model-1.safetensors"}}),
        )
        .unwrap();
        assert_eq!(files.len(), 1);
    }
}

// GGUF metadata usually puts the architecture and native context before tokenizer tables.
// Bound even malformed lengths and nesting; unknown metadata simply leaves context unreported.
pub(super) fn gguf_context(path: &std::path::Path) -> Result<Option<u64>, String> {
    use std::io::Read;
    let mut header = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(2 * 1024 * 1024)
        .read_to_end(&mut header)
        .map_err(|e| e.to_string())?;
    if !header.starts_with(b"GGUF") || header.len() < 24 {
        return Err("Invalid GGUF model header".into());
    }
    let mut bytes = &header[4..];
    let version = integer(&mut bytes, 4).unwrap();
    if !matches!(version, 2 | 3) {
        return Err("Unsupported GGUF model version".into());
    }
    let _tensors = integer(&mut bytes, 8);
    let fields = integer(&mut bytes, 8).unwrap_or(0).min(100_000);
    for _ in 0..fields {
        let Some(key) = gguf_string(&mut bytes) else {
            break;
        };
        let Some(kind) = integer(&mut bytes, 4) else {
            break;
        };
        if key.ends_with(b".context_length") {
            let value = match kind {
                2 | 3 => integer(&mut bytes, 2),
                4 | 5 => integer(&mut bytes, 4),
                10 | 11 => integer(&mut bytes, 8),
                _ => None,
            };
            return Ok(value.filter(|n| *n > 0 && *n <= u32::MAX as u64));
        }
        if skip_gguf(&mut bytes, kind).is_none() {
            break;
        }
    }
    Ok(None)
}
fn integer(bytes: &mut &[u8], width: usize) -> Option<u64> {
    let (value, next) = bytes.split_at_checked(width)?;
    let mut word = [0; 8];
    word[..width].copy_from_slice(value);
    *bytes = next;
    Some(u64::from_le_bytes(word))
}
fn gguf_string<'a>(bytes: &mut &'a [u8]) -> Option<&'a [u8]> {
    let n = usize::try_from(integer(bytes, 8)?).ok()?;
    let (value, next) = bytes.split_at_checked(n)?;
    *bytes = next;
    Some(value)
}
fn skip_gguf(bytes: &mut &[u8], kind: u64) -> Option<()> {
    fn width(kind: u64) -> Option<usize> {
        match kind {
            0 | 1 | 7 => Some(1),
            2 | 3 => Some(2),
            4..=6 => Some(4),
            10..=12 => Some(8),
            _ => None,
        }
    }
    let n = match kind {
        8 => {
            gguf_string(bytes)?;
            return Some(());
        }
        9 => {
            let item = integer(bytes, 4)?;
            let count = usize::try_from(integer(bytes, 8)?).ok()?;
            if item == 8 {
                if count > bytes.len() / 8 {
                    return None;
                }
                for _ in 0..count {
                    gguf_string(bytes)?;
                }
                return Some(());
            }
            width(item)?.checked_mul(count)?
        }
        _ => width(kind)?,
    };
    *bytes = bytes.get(n..)?;
    Some(())
}
