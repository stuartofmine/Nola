#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Diagnostic only: compare text, native PDF and rendered-image input.

Requires PyMuPDF in the invoking Python environment. Uses a synthetic document,
never benchmark answers. Configuration credentials remain in memory. A correct
response demonstrates this fixture only, not general PDF or music competence.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import secrets
import urllib.error
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    # Refuse overwriting evidence before issuing any requests.
    if args.output.exists():
        parser.error("output already exists")
    import pymupdf

    config = json.loads(args.config.read_text())["llm"]
    code = str(secrets.randbelow(900000) + 100000)
    document = pymupdf.open()
    page = document.new_page(width=400, height=200)
    page.insert_text((30, 75), f"Verification code: {code}", fontsize=24)
    pdf = document.tobytes()
    png = page.get_pixmap(matrix=pymupdf.Matrix(2, 2)).tobytes("png")
    data_url = lambda mime, data: f"data:{mime};base64," + base64.b64encode(data).decode()
    arms = {
        "text_control": {"type": "text", "text": f"Verification code: {code}"},
        "native_pdf": {"type": "file", "file": {"filename": "probe.pdf",
            "file_data": data_url("application/pdf", pdf)}},
        "rendered_png": {"type": "image_url", "image_url": {"url": data_url("image/png", png)}},
    }
    endpoint = config["endpoint"].rstrip("/")
    if not endpoint.endswith("/chat/completions"):
        endpoint += "/chat/completions"
    report = {"model": config["model"], "official_benchmark": False,
        "fixture_sha256": hashlib.sha256(pdf).hexdigest(), "expected": code,
        "interpretation": "one synthetic fixture; no general capability certification", "arms": {}}
    for name, block in arms.items():
        body = {"model": config["model"], "stream": False, "max_tokens": 64,
            "enable_thinking": False, "chat_template_kwargs": {"enable_thinking": False},
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "Read the verification code in the attached content. Return only its six digits. If unavailable return UNAVAILABLE; do not guess."}, block]}]}
        request = urllib.request.Request(endpoint, data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json", "Authorization": "Bearer " + config["api_key"]})
        try:
            with urllib.request.urlopen(request, timeout=45) as response:
                result = json.load(response)
            answer = result["choices"][0]["message"].get("content") or ""
            report["arms"][name] = {"answer": answer, "exact_match": answer.strip() == code}
        except urllib.error.HTTPError as error:
            # Error bodies may echo credentials/request bodies; retain code only.
            report["arms"][name] = {"http_status": error.code, "exact_match": False}
        except (OSError, ValueError, KeyError, IndexError) as error:
            report["arms"][name] = {"error_type": type(error).__name__, "exact_match": False}
        print(name, json.dumps(report["arms"][name]), flush=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as handle:
        json.dump(report, handle, indent=2)
        handle.write("\n")


if __name__ == "__main__":
    main()
