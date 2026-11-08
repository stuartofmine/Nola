#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real-model missing-renderer recovery in a fresh disposable container (not benchmark scoring)."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import time

from run_pdf_page_smoke import final_journal_answers


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--entrypoint", choices=["cli", "agent-host"], default="cli")
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--image", default="python:3.13-slim")
    parser.add_argument("--seconds", type=int, default=240)
    parser.add_argument("--prompt", default=(
        "查看 /work/card.pdf 第一页，最后只回复其中的六位验证码。先用 read_file 的 pdf_page=1 "
        "尝试读取；若环境缺少依赖，请准备环境、验证后继续。不要用 PDF 文本提取替代看图。"))
    args = parser.parse_args()
    if args.seconds <= 0:
        parser.error("--seconds must be positive")
    for binary in ["nola", "nola-agent-host"]:
        if not (args.runtime / binary).is_file():
            parser.error(f"Incomplete product runtime: missing {binary}")
    import pymupdf
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    project = root / "project"
    project.mkdir()
    (project / ".git").mkdir()
    code = str(secrets.randbelow(900000) + 100000)
    source = pymupdf.open()
    page = source.new_page(width=400, height=200)
    page.insert_text((30, 75), f"Code: {code}", fontsize=30)
    # Raster-only PDF: text extraction cannot satisfy the visual check.
    document = pymupdf.open()
    document.new_page(width=400, height=200).insert_image(
        page.rect, stream=page.get_pixmap().tobytes("png"))
    document.save(project / "card.pdf")
    config = project / "nola.json"
    llm = json.loads(args.config.read_text())["llm"]
    with open(config, "x", opener=lambda path, flags: os.open(path, flags, 0o600)) as handle:
        json.dump({"llm":llm, "allowed_dirs":["/work", "/tmp"],
            "model_capabilities":{"vision_input":True,"native_pdf_input":False},
            "max_iterations":0, "instructions":[
                "This is a disposable test container. You may install task-required dependencies "
                "inside it, including system packages. Do not change host files outside /work, "
                "bypass permissions or network restrictions. All setup counts toward the deadline."
            ]}, handle)
    shutil.copyfile(Path(__file__).resolve().parents[1] / "evals/nola_agent_supervisor.py",
                    project / "supervisor.py")
    name = "nola-env-smoke-" + secrets.token_hex(6)
    mounts = ["--mount", f"type=bind,src={args.runtime.resolve()},dst=/opt/nola,readonly",
              "--mount", f"type=bind,src={project},dst=/work"]
    started = time.monotonic()
    try:
        preflight = subprocess.run(["docker", "run", "--rm", args.image,
            "python3", "-c", "import shutil,json;print(json.dumps({'pdftoppm':shutil.which('pdftoppm')}))"],
            capture_output=True, text=True, timeout=60, check=True)
        observed = json.loads(preflight.stdout)
        if observed["pdftoppm"] is not None:
            raise RuntimeError("Fixture must start without the renderer")
        # Both product entrypoints consume the same configuration and Core/Host
        # implementation. No benchmark tool-set or shell-contract override.
        if args.entrypoint == "agent-host":
            agent = ["/opt/nola/nola-agent-host", "run", "--host-request-stdin",
                     "--session", name, "--project", "/work", "--json"]
            agent_input = json.dumps({"prompt":args.prompt,
                "permission_profile":"allow_all", "max_iterations":0}) + "\n"
            # Stand in for the native client in this isolated integration test.
            # Bind an explicit, limited tool-policy fixture to this exact session;
            # do not disable or modify the product's receipt validation.
            permitted = ["read_file", "environment_probe", "bash", "write_file",
                         "edit_file", "tool_search", "tool_output_read", "tool_output_find"]
            digest = hashlib.sha256(json.dumps(permitted).encode()).hexdigest()
            receipt = {"policy_digest":digest,"manifest_digest":digest,
                "expected_identity":{"client_id":"mac","session_id":name},
                "manifest_valid_until_ms":int(time.time() * 1000) + (args.seconds + 60) * 1000,
                "enabled_tools":[{"tool_id":tool,"enabled":True,"route":"agent_host"}
                                 for tool in permitted]}
            transport_env = ["-e", "NOLA_CORE_TOOL_POLICY_RECEIPT=" +
                base64.b64encode(json.dumps(receipt).encode()).decode(),
                "-e", "NOLA_CORE_TOOL_POLICY_DIGEST=" + digest,
                "-e", "NOLA_CORE_TOOL_MANIFEST_DIGEST=" + digest]
        else:
            agent = ["/opt/nola/nola", "--project", "/work", "--json", "--yes",
                     "--max-iterations", "0"]
            agent_input = args.prompt
            transport_env = []
        with (root / "events.ndjson").open("x") as output:
            result = subprocess.run(["docker", "run", "--rm", "-i", "--name", name,
                *mounts, *transport_env, "-w", "/work", "-e", "NOLA_DB_PATH=/work/state.db",
                "-e", "NOLA_SKIP_ONBOARDING=1", args.image, "python3", "/work/supervisor.py",
                "--seconds", str(args.seconds), "--", *agent],
                input=agent_input, text=True, stdout=output, stderr=subprocess.STDOUT,
                timeout=args.seconds + 30)
    finally:
        # Exact container created by this smoke only; no unrelated containers.
        subprocess.run(["docker", "rm", "-f", name], capture_output=True, timeout=30)
        config.unlink(missing_ok=True)
    answers = final_journal_answers(project / "state.db") if (project / "state.db").exists() else []
    calls = []
    for line in (root / "events.ndjson").read_text().splitlines():
        try:
            trace = json.loads(line).get("trace", {})
        except ValueError:
            continue
        if trace.get("kind") == "tool.completed":
            payload = trace.get("payload", {})
            calls.append({"name":payload.get("tool_name"),"status":trace.get("status"),
                "arguments":payload.get("arguments"),"detail":payload.get("detail")})
    report = {"official_benchmark":False,"entrypoint":args.entrypoint,"initial_environment":observed,
        "native_policy_receipt":"session-bound integration fixture" if args.entrypoint == "agent-host" else None,
        "elapsed_seconds":time.monotonic()-started,"exit_code":result.returncode,
        "expected":code,"answers":answers,"answer_matches":code in [a.strip() for a in answers],
        "tools":calls}
    (root / "summary.json").write_text(json.dumps(report, ensure_ascii=False, indent=2))
    print(json.dumps({k:v for k,v in report.items() if k != "tools"}, ensure_ascii=False))


if __name__ == "__main__":
    main()
