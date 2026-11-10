---
name: install-open-source-model
description: Find, download, configure, and validate an open-source language or multimodal model variant that fits the current device. Use when the user names a model or repository and wants Nola to choose among MLX, GGUF, quantized, or other published variants based on hardware, context length, memory, disk, architecture, and available runtimes.
---

# Install an open-source model

Turn a model name into one verified local runtime configuration. Never equate a popular quantization, a completed download, or a file existing on disk with a usable model.

## Readiness

- Preserve the user's original model identity, license constraints, required modalities, minimum context, quality preference, and storage limit.
- Inspect the actual hardware profile: platform, CPU architecture, GPU vendor, accelerator backend, GPU or unified memory, system memory, free disk, and installed runtimes. Support Apple Metal/MLX, NVIDIA CUDA, AMD ROCm, AMD Vulkan, and CPU fallback. Default to at least 64K context when the user gives no lower requirement.
- Discover variants from the model publisher and trusted conversion publishers. Use the original model query when searching; do not route from isolated keywords.
- Treat repository metadata, file sizes, architecture, quantization, tokenizer files, context configuration, and license as evidence. A repository name alone is insufficient.

## Workflow

1. Resolve the canonical upstream model and record publisher, revision, license, parameter count, modalities, architecture, and native context limit.
2. Collect compatible variants. Prefer native MLX on Apple silicon, CUDA-backed llama.cpp on NVIDIA, ROCm-backed llama.cpp on supported AMD systems, Vulkan-backed llama.cpp as the AMD fallback, and CPU llama.cpp only when no supported accelerator is available. Never relabel an incompatible architecture, accelerator, or runtime as compatible.
3. Write a hardware JSON snapshot and candidate JSON ledger, then run:

   ```bash
   python3 scripts/plan_model_variant.py \
     --model '<canonical model id>' \
     --hardware-json '<hardware.json>' \
     --candidates-json '<candidates.json>' \
     --minimum-context 65536 \
     --output '<model-install-plan.json>'
   ```

4. Present the recommended variant, format, quantization, download size, estimated peak memory, context, headroom, source revision, rejected candidates, and material quality tradeoff. If no candidate fits, stop instead of silently lowering the required context or oversubscribing memory.
5. Confirm approval immediately before network download or persistent runtime configuration unless the user's current request already explicitly authorizes both actions. Reuse the Host permission decision; do not ask repeatedly for each shard.
6. Model downloads are intentionally long-running work. Start `scripts/install_model_variant.py` with the formal `bash` tool using `execution_mode: background`; retain the returned task ID and show it to the user. Never hold a foreground model turn open for the download, repeatedly poll it, or impose the normal short command timeout. Use `bash_background_status` when the user asks for progress, `bash_background_wait` only for a bounded wait, and `bash_background_stop` to cancel.
7. Download only the selected immutable revision into the platform model cache. Build an `artifacts.json` object with the immutable revision and every required tokenizer, configuration, index, and weight shard (`path`, `url`, positive `size_bytes`, and published `sha256`/`etag`). A single weight file is insufficient for a multi-file MLX or safetensors repository. Invoke `scripts/install_model_variant.py --artifacts-json '<artifacts.json>'` without inventing a model root; the active Host supplies `NOLA_MODELS_ROOT`. The legacy single-artifact and explicit root arguments are only for fixtures and genuinely single-file GGUF releases. The installer resumes safely from persistent `.part` files, binds partials to remote validators, validates `Content-Range`, uses per-model and shared-catalog cross-process locks with stale-owner recovery, reserves disk, verifies the complete manifest with exact byte counts and SHA-256 where published, and atomically activates a snapshot only after every artifact passes. Keep `task.json` updated with the stable model task ID, status, current artifact, aggregate bytes, speed, ETA, attempt count, and heartbeat time. Never guess a second cache location.
8. Register the model through the platform's supported model configuration surface. The installed artifacts, `model.json`, and the atomic `catalog.json` entry must all live under the Nola models root. Re-open the product model directory and prove that it consumed the new catalog entry; merely writing `catalog.json` is not discovery. Do not claim configuration when the active client cannot load dynamic catalog entries or the active runtime cannot consume the selected format.
9. Start the runtime only when explicitly requested or as a bounded validation step. Run tokenizer load, model load, required-context allocation, and a deterministic smoke inference; then stop a validation-only runtime.

## User control

- Search, metadata reads, and dry-run planning are read-only. Downloading consumes network and disk; configuration changes persistent state; starting a runtime consumes compute and memory.
- A background task acknowledgement is not completion. Report it as running, preserve its task ID, and let the user continue chatting while it runs.
- Show the exact repository, revision, files, total bytes, cache destination, runtime, and replacement behavior before the first side effect.
- Never install a dependency, overwrite another model, delete a cache, accept gated terms, or execute remote repository code without separate explicit authority.
- Prefer a reversible disabled registration. Selecting a model must not automatically start it.

## Validation

- Re-run the planner against the final hardware and selected candidate; the selected entry must remain `compatible`.
- Verify manifest completeness, byte totals, checksums where available, tokenizer/config compatibility, architecture, quantization, and license metadata.
- Re-open the Nola model catalog and verify the installed model appears with the selected accelerator (`metal`, `cuda`, `rocm`, `vulkan`, or `cpu`) and an existing artifact path.
- Require a real runtime load and smoke inference before reporting `Ready`. A downloaded but unloaded model is only `Downloaded`; a registered but stopped model is `Configured`.
- Record observed context allocation and peak memory when the runtime exposes them. Label heuristic estimates as estimates.

## Completion

Report canonical model, selected variant and revision, format and quantization, context, download and estimated runtime footprint, cache path, runtime, configuration state, validation evidence, and any unsupported feature. Include rejected alternatives only when their rejection explains the recommendation.
