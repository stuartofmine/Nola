#!/usr/bin/env python3
"""Create and validate an auditable, unsent RFC 5322 email draft."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import re
import sys
from email import policy
from email.message import EmailMessage
from email.parser import BytesParser
from email.utils import format_datetime, getaddresses, parsedate_to_datetime, parseaddr
from pathlib import Path
from typing import Any


def fail(message: str) -> None:
    raise ValueError(message)


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        fail("input must be a JSON object")
    return value


def clean_header(value: Any, field: str) -> str:
    text = str(value or "").strip()
    if not text or "\r" in text or "\n" in text:
        fail(f"invalid {field}")
    return text


def address(value: Any, field: str) -> str:
    text = clean_header(value, field)
    _, parsed = parseaddr(text)
    if not parsed or "@" not in parsed:
        fail(f"invalid {field} address")
    return parsed.lower()


def addresses(value: Any) -> list[str]:
    values = value if isinstance(value, list) else [value]
    parsed = [item.lower() for _, item in getaddresses([str(item) for item in values]) if "@" in item]
    return list(dict.fromkeys(parsed))


def stable_id(prefix: str, parts: list[str]) -> str:
    digest = hashlib.sha256("\n".join(parts).encode("utf-8")).hexdigest()[:20]
    return f"{prefix}-{digest}"


def normalized_subject(subject: str) -> str:
    return subject if re.match(r"(?i)^re\s*:", subject) else f"Re: {subject}"


def build_draft(source: dict[str, Any]) -> tuple[EmailMessage, dict[str, Any]]:
    account = source.get("account")
    messages = source.get("messages")
    request = source.get("requested_reply")
    if not isinstance(account, dict) or not isinstance(request, dict):
        fail("account and requested_reply are required")
    if not isinstance(messages, list) or not messages or not isinstance(messages[-1], dict):
        fail("messages must contain an ordered thread")

    own_address = address(account.get("email"), "account")
    sender_name = clean_header(account.get("display_name") or own_address, "display name")
    latest = messages[-1]
    recipient = address(latest.get("from"), "latest sender")
    thread_id = clean_header(source.get("thread_id"), "thread id")
    source_message_id = clean_header(latest.get("message_id"), "message id")
    subject = normalized_subject(clean_header(source.get("subject"), "subject"))

    reply_all_candidates: list[str] = []
    if request.get("reply_all") is True:
        reply_all_candidates = addresses(latest.get("to", [])) + addresses(latest.get("cc", []))
    excluded_cc = sorted({item for item in reply_all_candidates if item not in {own_address, recipient}})

    requested_attachments = [clean_header(item, "requested attachment") for item in request.get("attachments", [])]
    available = {clean_header(item, "available attachment") for item in source.get("available_attachments", [])}
    missing_attachments = [item for item in requested_attachments if item not in available]

    facts = request.get("grounded_facts", [])
    if not isinstance(facts, list) or not facts:
        fail("requested_reply.grounded_facts must be a non-empty list")
    grounded_facts = [str(item).strip() for item in facts if str(item).strip()]
    if not grounded_facts:
        fail("grounded facts cannot be empty")

    fallback_name = parseaddr(str(latest.get("from", "")))[0] or "there"
    greeting_name = clean_header(request.get("recipient_name") or fallback_name, "recipient name")
    closing = clean_header(request.get("closing") or sender_name, "closing")
    paragraphs = [f"Hello {greeting_name},", "", *grounded_facts]
    if missing_attachments:
        paragraphs.extend([
            "",
            "The requested attachment is not available yet, so this draft is not ready to send.",
        ])
    paragraphs.extend(["", f"Best,\n{closing}"])
    body = "\n".join(paragraphs)

    blockers = []
    if excluded_cc:
        blockers.append("reply-all-review")
    if missing_attachments:
        blockers.append("missing-attachment")

    correlation_key = stable_id("mail", [thread_id, subject, recipient, body])
    draft_id = stable_id("draft", [thread_id, correlation_key])
    message = EmailMessage(policy=policy.SMTP)
    message["From"] = f"{sender_name} <{own_address}>"
    message["To"] = recipient
    message["Date"] = format_datetime(datetime.now(timezone.utc))
    message["Subject"] = subject
    message["In-Reply-To"] = source_message_id
    message["References"] = source_message_id
    message["X-Nola-Thread-ID"] = thread_id
    message["X-Nola-Draft-ID"] = draft_id
    message["X-Nola-Correlation-Key"] = correlation_key
    message["X-Nola-Draft"] = "true"
    message["X-Nola-Sent"] = "false"
    message["X-Nola-Send-Blocked"] = ",".join(blockers) if blockers else "none"
    if excluded_cc:
        message["X-Nola-Excluded-Cc"] = ",".join(excluded_cc)
    if missing_attachments:
        message["X-Nola-Missing-Attachments"] = ",".join(missing_attachments)
    message.set_content(body)

    receipt = {
        "validated": True,
        "mode": "draft_only",
        "draft_id": draft_id,
        "correlation_key": correlation_key,
        "thread_id": thread_id,
        "sent": False,
        "send_blocked": bool(blockers),
        "blockers": blockers,
        "to": [recipient],
        "cc": [],
        "excluded_cc": excluded_cc,
        "missing_attachments": missing_attachments,
    }
    return message, receipt


def validate_draft(path: Path) -> dict[str, Any]:
    if not path.is_file() or path.stat().st_size < 300:
        fail("draft is missing or empty")
    message = BytesParser(policy=policy.default).parsebytes(path.read_bytes())
    required = ["From", "To", "Date", "Subject", "In-Reply-To", "References", "X-Nola-Thread-ID", "X-Nola-Draft-ID", "X-Nola-Correlation-Key"]
    missing = [field for field in required if not message.get(field)]
    if missing:
        fail("missing headers: " + ", ".join(missing))
    try:
        if parsedate_to_datetime(str(message["Date"])) is None:
            fail("invalid Date header")
    except (TypeError, ValueError, OverflowError):
        fail("invalid Date header")
    if str(message.get("X-Nola-Draft", "")).lower() != "true":
        fail("message is not marked as a draft")
    if str(message.get("X-Nola-Sent", "")).lower() != "false":
        fail("draft incorrectly claims a send")
    if message.get("Message-ID") or message.get("X-Provider-Message-ID"):
        fail("unsent draft must not contain a provider message identifier")
    body = message.get_body(preferencelist=("plain",))
    text = body.get_content() if body is not None else message.get_content()
    if not text.strip():
        fail("draft body is empty")
    blockers = [item for item in str(message.get("X-Nola-Send-Blocked", "none")).split(",") if item and item != "none"]
    return {
        "validated": True,
        "mode": "draft_only",
        "draft_id": str(message["X-Nola-Draft-ID"]),
        "correlation_key": str(message["X-Nola-Correlation-Key"]),
        "thread_id": str(message["X-Nola-Thread-ID"]),
        "sent": False,
        "send_blocked": bool(blockers),
        "blockers": blockers,
        "to": addresses(message.get_all("To", [])),
        "cc": addresses(message.get_all("Cc", [])),
        "excluded_cc": addresses(message.get("X-Nola-Excluded-Cc", "")),
        "missing_attachments": [item for item in str(message.get("X-Nola-Missing-Attachments", "")).split(",") if item],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--validate-only", action="store_true")
    args = parser.parse_args()
    try:
        if args.validate_only:
            receipt = validate_draft(args.output)
        else:
            if args.input is None:
                fail("--input is required when creating a draft")
            message, receipt = build_draft(load_json(args.input))
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_bytes(message.as_bytes(policy=policy.SMTP))
            receipt = validate_draft(args.output) | receipt
        print(json.dumps(receipt, ensure_ascii=True, sort_keys=True))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(json.dumps({"validated": False, "error": str(error)}, ensure_ascii=True), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
