#!/usr/bin/env python3
"""Select a local model artifact from explicit hardware and candidate evidence."""

import argparse
import json
from pathlib import Path


def load_object(path):
    value = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def positive_int(value, field):
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise ValueError(f"{field} must be a positive integer")
    return value


def normalized_runtime(candidate):
    runtime = str(candidate.get("runtime", "")).strip().lower()
    if runtime:
        return runtime
    return "mlx" if str(candidate.get("format", "")).lower() == "mlx" else "llama.cpp"


def estimate_peak_bytes(candidate, context_tokens):
    explicit = candidate.get("estimated_peak_memory_bytes")
    if explicit is not None:
        return positive_int(explicit, "estimated_peak_memory_bytes")
    weights = positive_int(candidate.get("size_bytes"), "size_bytes")
    parameters = float(candidate.get("parameter_billions", 1.0))
    # Conservative planning heuristic: runtime overhead plus a context-sensitive KV allowance.
    kv_bytes_per_token = 64 * 1024 if parameters <= 1.5 else 128 * 1024 if parameters <= 7 else 256 * 1024
    return int(weights * 1.35 + context_tokens * kv_bytes_per_token)


def normalized_accelerators(hardware):
    explicit = {str(value).strip().lower() for value in hardware.get("accelerators", [])}
    vendor = str(hardware.get("gpu_vendor", "")).strip().lower()
    platform = str(hardware.get("platform", "")).strip().lower()
    architecture = str(hardware.get("architecture", "")).strip().lower()
    if vendor in {"apple", "metal"} or (platform in {"macos", "darwin"} and architecture in {"arm64", "aarch64"}):
        explicit.add("metal")
    elif vendor in {"nvidia", "cuda"}:
        explicit.add("cuda")
    elif vendor in {"amd", "advanced micro devices"}:
        # ROCm support varies by OS and GPU. Only advertise it when hardware discovery did.
        if hardware.get("rocm_available"):
            explicit.add("rocm")
        if hardware.get("vulkan_available"):
            explicit.add("vulkan")
    explicit.add("cpu")
    return explicit


def selected_accelerator(candidate, hardware):
    available = normalized_accelerators(hardware)
    supported = [str(value).strip().lower() for value in candidate.get("accelerators", [])]
    if not supported:
        supported = ["metal"] if str(candidate.get("format", "")).lower() == "mlx" else ["cuda", "rocm", "vulkan", "cpu"]
    preference = ["metal", "cuda", "rocm", "vulkan", "cpu"]
    return next((value for value in preference if value in available and value in supported), None)


def evaluate(candidate, hardware, minimum_context):
    reasons = []
    runtime = normalized_runtime(candidate)
    runtimes = {str(value).lower() for value in hardware.get("runtimes", [])}
    architecture = str(hardware.get("architecture", "")).lower()
    supported_architectures = {str(value).lower() for value in candidate.get("architectures", [])}
    context = positive_int(candidate.get("context_length"), "context_length")
    size = positive_int(candidate.get("size_bytes"), "size_bytes")
    memory = positive_int(hardware.get("memory_bytes"), "memory_bytes")
    disk = positive_int(hardware.get("available_disk_bytes"), "available_disk_bytes")
    peak = estimate_peak_bytes(candidate, minimum_context)
    accelerator = selected_accelerator(candidate, hardware)
    accelerator_memory = hardware.get("gpu_memory_bytes") if accelerator in {"cuda", "rocm", "vulkan"} else None
    usable_memory = positive_int(accelerator_memory, "gpu_memory_bytes") if accelerator_memory is not None else memory

    if runtime not in runtimes:
        reasons.append(f"runtime {runtime} is unavailable")
    if supported_architectures and architecture not in supported_architectures:
        reasons.append(f"architecture {architecture} is unsupported")
    if accelerator is None:
        reasons.append("no supported accelerator backend is available")
    if context < minimum_context:
        reasons.append(f"context {context} is below required {minimum_context}")
    if peak > int(usable_memory * 0.80):
        memory_kind = "GPU memory" if accelerator_memory is not None else "device memory"
        reasons.append(f"estimated peak memory exceeds 80% of {memory_kind}")
    if size > int(disk * 0.85):
        reasons.append("download exceeds 85% of currently available disk")

    platform = str(hardware.get("platform", "")).lower()
    format_name = str(candidate.get("format", "")).lower()
    accelerator_bonus = {"metal": 35, "cuda": 32, "rocm": 30, "vulkan": 22, "cpu": 0}.get(accelerator, 0)
    native_bonus = 10 if platform == "macos" and architecture == "arm64" and format_name == "mlx" else 0
    headroom = max(usable_memory - peak, 0)
    score = native_bonus + accelerator_bonus + min(context / minimum_context, 4) * 10 + headroom / usable_memory * 20 - size / disk * 5
    result = dict(candidate)
    result.update({
        "runtime": runtime,
        "accelerator": accelerator,
        "estimated_peak_memory_bytes": peak,
        "memory_headroom_bytes": headroom,
        "compatible": not reasons,
        "rejection_reasons": reasons,
        "score": round(score, 3),
    })
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True)
    parser.add_argument("--hardware-json", required=True)
    parser.add_argument("--candidates-json", required=True)
    parser.add_argument("--minimum-context", type=int, default=65536)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()

    if args.minimum_context < 1024:
        raise ValueError("minimum context must be at least 1024 tokens")
    hardware = load_object(args.hardware_json)
    ledger = load_object(args.candidates_json)
    candidates = ledger.get("candidates")
    if not isinstance(candidates, list) or not candidates:
        raise ValueError("candidates.json must contain a non-empty candidates array")

    evaluated = [evaluate(candidate, hardware, args.minimum_context) for candidate in candidates]
    compatible = sorted((item for item in evaluated if item["compatible"]), key=lambda item: item["score"], reverse=True)
    rejected = sorted((item for item in evaluated if not item["compatible"]), key=lambda item: item["score"], reverse=True)
    plan = {
        "schema_version": 1,
        "model": args.model,
        "minimum_context_tokens": args.minimum_context,
        "hardware": hardware,
        "status": "recommended" if compatible else "no_compatible_variant",
        "selected": compatible[0] if compatible else None,
        "alternatives": compatible[1:4],
        "rejected": rejected,
        "side_effects_performed": False,
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(plan, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"status": plan["status"], "selected": plan["selected"]}, ensure_ascii=False))
    if not compatible:
        raise SystemExit(2)


if __name__ == "__main__":
    main()
