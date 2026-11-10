import json
import datetime as dt
import hashlib
import os
import re
import shutil
import socket
import subprocess
import threading
import time
import uuid
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from urllib.parse import urlsplit

import pytest
from playwright.sync_api import expect, sync_playwright


REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
CLI_ROOT = REPOSITORY_ROOT / "apps/cli"
NOLA_BINARY = REPOSITORY_ROOT / "target" / "debug" / "nola"
WEB_ROOT = REPOSITORY_ROOT / "apps/web"
sys.path.insert(0, str(REPOSITORY_ROOT / "scripts"))
from real_e2e_evidence import record_step


def unused_local_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def wait_for_http(url: str, timeout: float = 15.0) -> None:
    import urllib.request

    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=0.5) as response:
                if response.status == 200:
                    return
        except Exception:
            pass
        time.sleep(0.05)
    raise AssertionError(f"server did not become ready: {url}")


def require_real_llm_configuration() -> tuple[Path, dict]:
    if os.environ.get("NOLA_RUN_REAL_MODEL_E2E") != "1":
        pytest.skip("set NOLA_RUN_REAL_MODEL_E2E=1 to run the real-model Web gate")
    fixture = os.environ.get("NOLA_REAL_LLM_CONFIG", "").strip()
    path = Path(fixture).expanduser() if fixture else Path.home() / ".nola" / "config.json"
    if not path.is_file():
        pytest.fail(f"real-model gate was requested but configuration is missing: {path}")
    try:
        config = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        pytest.fail(f"cannot read saved CLI configuration {path}: {error}")
    llm = config.get("llm") if isinstance(config, dict) else None
    if not isinstance(llm, dict) or not str(llm.get("endpoint") or "").strip():
        pytest.fail(f"saved CLI configuration has no llm.endpoint: {path}")
    if not str(llm.get("model") or "").strip():
        pytest.fail(f"saved CLI configuration has no llm.model: {path}")
    return path, config


def install_cli_configuration(
    source: Path, config: dict, home: Path, endpoint: str | None = None
) -> None:
    destination = home / ".nola" / "config.json"
    destination.parent.mkdir(parents=True, exist_ok=True)
    if endpoint is None:
        shutil.copyfile(source, destination)
        return
    isolated = json.loads(json.dumps(config))
    isolated["llm"]["endpoint"] = endpoint
    destination.write_text(json.dumps(isolated, indent=2) + "\n", encoding="utf-8")


def web_access_token_path(home: Path) -> Path:
    """Return the credential path owned and reported by the Web Host."""
    return home / ".nola" / "web" / "mobile-access-token"


class UnknownToolFaultProxy:
    """Corrupt one real-model tool name, then relay recovery traffic unchanged."""

    def __init__(self, upstream: str, log_path: Path, corrupt_first_tool: bool = True) -> None:
        self.upstream = upstream.rstrip("/")
        self.upstream_host = urlsplit(self.upstream).hostname or ""
        self.log_path = log_path
        self.corrupt_first_tool = corrupt_first_tool
        self.request_count = 0
        self.original_tool_name: str | None = None
        self._lock = threading.Lock()
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    @property
    def endpoint(self) -> str:
        return f"http://127.0.0.1:{self._server.server_address[1]}/v1"

    def start(self) -> None:
        self._thread.start()

    def close(self) -> None:
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)

    def _target(self, incoming: str) -> str:
        _, separator, query = incoming.partition("?")
        target = (
            self.upstream
            if self.upstream.endswith("/chat/completions")
            else f"{self.upstream}/chat/completions"
        )
        return f"{target}?{query}" if separator else target

    def _rewrite_event(self, event: dict) -> None:
        if not self.corrupt_first_tool:
            return
        with self._lock:
            if self.original_tool_name is not None:
                return
            for choice in event.get("choices", []):
                container = choice.get("delta") or choice.get("message") or {}
                for call in container.get("tool_calls") or []:
                    function = call.get("function") or {}
                    name = function.get("name")
                    if isinstance(name, str) and name:
                        self.original_tool_name = name
                        function["name"] = "nola_unknown_tool_probe"
                        return

    def _rewrite(self, body: bytes, content_type: str) -> bytes:
        if "text/event-stream" not in content_type:
            try:
                event = json.loads(body)
            except json.JSONDecodeError:
                return body
            self._rewrite_event(event)
            return json.dumps(event, separators=(",", ":")).encode()
        output: list[bytes] = []
        for line in body.splitlines(keepends=True):
            if line.startswith(b"data: ") and line.strip() != b"data: [DONE]":
                newline = b"\n" if line.endswith(b"\n") else b""
                try:
                    event = json.loads(line[6:].rstrip(b"\r\n"))
                except json.JSONDecodeError:
                    output.append(line)
                    continue
                self._rewrite_event(event)
                line = b"data: " + json.dumps(event, separators=(",", ":")).encode() + newline
            output.append(line)
        return b"".join(output)

    def _handler(self):
        proxy = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, _format: str, *_args) -> None:
                return

            def do_POST(self) -> None:
                from http.client import IncompleteRead

                started = time.monotonic()
                body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                try:
                    request_value = json.loads(body)
                except json.JSONDecodeError:
                    request_value = {}
                headers = {
                    key: value
                    for key, value in self.headers.items()
                    if key.lower() not in {"host", "content-length", "connection", "accept-encoding"}
                }
                with proxy._lock:
                    proxy.request_count += 1
                request = Request(proxy._target(self.path), data=body, headers=headers, method="POST")
                try:
                    response = urlopen(request, timeout=180)
                except HTTPError as error:
                    response = error
                content_type = response.headers.get("Content-Type", "application/json")
                response_error = None
                try:
                    raw_payload = response.read()
                except IncompleteRead as error:
                    raw_payload = error.partial
                    response_error = f"IncompleteRead: {error}"
                payload = proxy._rewrite(raw_payload, content_type)
                self.send_response(response.status)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
                row = {
                    "schemaVersion": 1,
                    "event": "provider.response",
                    "requestId": uuid.uuid4().hex,
                    "startedAt": dt.datetime.now(dt.timezone.utc).isoformat(),
                    "durationMilliseconds": round((time.monotonic() - started) * 1000, 3),
                    "upstreamHost": proxy.upstream_host,
                    "upstreamPath": urlsplit(proxy._target(self.path)).path,
                    "status": response.status,
                    "responseBytes": len(payload),
                    "requestSha256": hashlib.sha256(body).hexdigest(),
                    "model": request_value.get("model"),
                    "stream": bool(request_value.get("stream")),
                    "error": response_error,
                }
                with proxy._lock:
                    proxy.log_path.parent.mkdir(parents=True, exist_ok=True)
                    with proxy.log_path.open("a", encoding="utf-8") as handle:
                        handle.write(json.dumps(row, separators=(",", ":")) + "\n")

        return Handler


def test_real_llm_web_login_stream_tool_trace_and_context(tmp_path: Path) -> None:
    config_path, saved_config = require_real_llm_configuration()
    if os.environ.get("NOLA_SKIP_REAL_WEB_BUILD") != "1":
        subprocess.run(
            ["cargo", "build", "--quiet", "--bin", "nola"],
            cwd=CLI_ROOT,
            check=True,
            timeout=300,
        )
    assert NOLA_BINARY.is_file(), f"Nola Web binary is missing: {NOLA_BINARY}"

    marker = f"NOLA-WEB-REAL-{uuid.uuid4().hex.upper()}"
    source_path = tmp_path / "release-note.txt"
    source_path.write_text(
        f"Internal release note\nUnique release code: {marker}\n",
        encoding="utf-8",
    )
    home = tmp_path / "home"
    home.mkdir()
    provider_trace_path = tmp_path / "provider.ndjson"
    fault_proxy = UnknownToolFaultProxy(
        str(saved_config["llm"]["endpoint"]),
        provider_trace_path,
    )
    fault_proxy.start()
    install_cli_configuration(config_path, saved_config, home, fault_proxy.endpoint)
    web_port = unused_local_port()
    env = os.environ.copy()
    env["HOME"] = str(home)
    env["NOLA_DB_PATH"] = str(tmp_path / "nola.sqlite")
    server_log = (tmp_path / "web-host.log").open("w", encoding="utf-8")
    process = subprocess.Popen(
        [
            str(NOLA_BINARY),
            "web",
            "--host",
            "127.0.0.1",
            "--port",
            str(web_port),
            "--web-root",
            str(WEB_ROOT),
        ],
        cwd=CLI_ROOT,
        env=env,
        stdout=server_log,
        stderr=subprocess.STDOUT,
        text=True,
    )
    base_url = f"http://127.0.0.1:{web_port}"
    browser = None
    page = None
    step_recorded = False
    try:
        wait_for_http(f"{base_url}/login")
        token_path = web_access_token_path(home)
        deadline = time.monotonic() + 5
        while not token_path.is_file() and time.monotonic() < deadline:
            time.sleep(0.05)
        token = token_path.read_text(encoding="utf-8").strip()

        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(headless=True)
            page = browser.new_page()
            page_errors: list[str] = []
            page.on("pageerror", lambda error: page_errors.append(str(error)))
            page.goto(f"{base_url}/login?next=%2Fchat")
            page.locator("#hostToken").fill(token)
            page.locator('button[type="submit"]').click()
            page.wait_for_url(f"{base_url}/chat", wait_until="domcontentloaded")
            expect(page.locator("#chatMessages")).to_be_visible(timeout=10_000)
            page.evaluate(
                """() => {
                    localStorage.setItem('nola.onboarding.web.v2', 'complete');
                    const guide = document.getElementById('nolaGuide');
                    if (guide) guide.hidden = true;
                    document.body.style.overflow = '';
                }"""
            )
            page.evaluate(
                """async ({ outputDirectory }) => {
                    const response = await fetch('/api/agent/settings', {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({
                            enable_local_dev_tools: true,
                            host_tools_enabled: true,
                            allowed_dirs: [outputDirectory],
                            agent_mode_enabled: true,
                        }),
                    });
                    if (!response.ok) throw new Error(await response.text());
                }""",
                {"outputDirectory": str(tmp_path)},
            )
            page.reload(wait_until="domcontentloaded")
            page.wait_for_function("() => chatAgentModeEnabled === true", timeout=10_000)
            page.locator("#chatToolsMenu > summary").click()
            agent_badge = page.locator("#agentModeBadge")
            expect(agent_badge).to_contain_text(re.compile("开启|On"))
            expect(page.locator("#inspectorModeText")).to_have_text("Agent")

            prompt = (
                f"请查看 {source_path}，告诉我其中唯一的发布代号。"
                "先原样写出代号，再分别说明你从哪一行确认、文件共有几行、"
                "以及为什么这个答案不是猜测；使用一个 Markdown 标题，随后严格使用"
                "三行分别以 `- ` 开头的 Markdown 无序列表，不要改用普通段落。"
            )
            previous_assistant_count = page.locator(".chat-msg.msg-assistant").count()
            page.evaluate(
                """previousCount => {
                    window.__nolaE2EStreamTimeline?.observer?.disconnect?.();
                    const timeline = {
                        startedAt: performance.now(),
                        firstBubbleAt: null,
                        bubbleIds: [],
                        samples: [],
                    };
                    const sample = () => {
                        const messages = [...document.querySelectorAll('.chat-msg.msg-assistant')];
                        const bubble = messages[previousCount];
                        if (!bubble) return;
                        if (timeline.firstBubbleAt === null) timeline.firstBubbleAt = performance.now();
                        if (!bubble.dataset.e2eStreamIdentity) {
                            bubble.dataset.e2eStreamIdentity = `assistant-${crypto.randomUUID()}`;
                        }
                        if (!timeline.bubbleIds.includes(bubble.dataset.e2eStreamIdentity)) {
                            timeline.bubbleIds.push(bubble.dataset.e2eStreamIdentity);
                        }
                        const textRoot = bubble.querySelector('[data-role="text"]');
                        const text = textRoot?.innerText || '';
                        const previous = timeline.samples[timeline.samples.length - 1];
                            const next = {
                                at: performance.now(),
                                running: typeof isGeneratingResponse !== 'undefined'
                                    && Boolean(isGeneratingResponse),
                                textLength: text.length,
                            headingCount: textRoot?.querySelectorAll('h1,h2,h3,h4,h5,h6').length || 0,
                            listItemCount: textRoot?.querySelectorAll('li').length || 0,
                            codeBlockCount: textRoot?.querySelectorAll('pre').length || 0,
                        };
                        if (!previous
                            || previous.textLength !== next.textLength
                            || previous.headingCount !== next.headingCount
                            || previous.listItemCount !== next.listItemCount
                            || previous.codeBlockCount !== next.codeBlockCount) {
                            timeline.samples.push(next);
                        }
                    };
                    timeline.observer = new MutationObserver(sample);
                    timeline.observer.observe(document.getElementById('chatMessages'), {
                        subtree: true,
                        childList: true,
                        characterData: true,
                    });
                    window.__nolaE2EStreamTimeline = timeline;
                    sample();
                }""",
                previous_assistant_count,
            )
            page.locator(".chat-textarea").fill(prompt)
            page.locator("#sendBtn").click()
            expect(page.locator(".chat-msg.msg-assistant")).to_have_count(
                previous_assistant_count + 1,
                timeout=2_000,
            )
            assistant = page.locator(".chat-msg.msg-assistant").last
            expect(assistant).to_contain_text(marker, timeout=120_000)
            expect(assistant.locator('[data-permission="allow"]')).to_have_count(0)
            expect(assistant).to_have_attribute("data-trace-id", re.compile(r"trace_.+"))
            page.wait_for_function("() => !isGeneratingResponse", timeout=15_000)
            stream_timeline = page.evaluate(
                """() => {
                    const timeline = window.__nolaE2EStreamTimeline;
                    timeline?.observer?.disconnect?.();
                    if (!timeline) return null;
                    const { observer, ...serializable } = timeline;
                        const bubble = document.querySelector('[data-e2e-stream-identity]');
                        const textRoot = bubble?.querySelector('[data-role="text"]');
                        const finalSample = {
                            at: performance.now(),
                            running: typeof isGeneratingResponse !== 'undefined'
                                && Boolean(isGeneratingResponse),
                            textLength: (textRoot?.innerText || '').length,
                            headingCount: textRoot?.querySelectorAll('h1,h2,h3,h4,h5,h6').length || 0,
                            listItemCount: textRoot?.querySelectorAll('li').length || 0,
                            codeBlockCount: textRoot?.querySelectorAll('pre').length || 0,
                        };
                        const previous = timeline.samples[timeline.samples.length - 1];
                        if (!previous
                            || previous.textLength !== finalSample.textLength
                            || previous.headingCount !== finalSample.headingCount
                            || previous.listItemCount !== finalSample.listItemCount
                            || previous.codeBlockCount !== finalSample.codeBlockCount) {
                            timeline.samples.push(finalSample);
                        }
                        return {
                        ...serializable,
                        firstBubbleDelayMs: timeline.firstBubbleAt === null
                            ? null
                            : timeline.firstBubbleAt - timeline.startedAt,
                        finalText: textRoot?.innerText || '',
                        finalHTML: textRoot?.innerHTML || '',
                    };
                }"""
            )
            assert stream_timeline is not None
            positive_lengths = [
                int(sample["textLength"])
                for sample in stream_timeline["samples"]
                if int(sample["textLength"]) > 0
            ]
            assert stream_timeline["firstBubbleDelayMs"] < 2_000, stream_timeline
            assert len(stream_timeline["bubbleIds"]) == 1, stream_timeline
            assert len(set(positive_lengths)) >= 3, stream_timeline
            assert positive_lengths[-1] == max(positive_lengths), stream_timeline
            primary_tool_row = page.locator(".tool-activity-row").first
            expect(primary_tool_row).to_be_visible(timeout=5_000)
            # A file read is evidence inside the expandable tool trace, not a
            # deliverable attachment beneath the assistant response.
            expect(primary_tool_row.locator(".tool-artifact")).to_have_count(0)
            primary_tool_summary = primary_tool_row.locator(".tool-summary-copy").inner_text().strip()
            assert primary_tool_summary
            assert not primary_tool_summary.startswith("{"), primary_tool_summary
            assert len(primary_tool_summary) <= 360, primary_tool_summary
            technical_details = primary_tool_row.locator(".tool-technical-details")
            assert technical_details.count() == 1
            assert technical_details.get_attribute("open") is None
            primary_screenshot_path = tmp_path / "web-primary-success.png"
            page.screenshot(path=str(primary_screenshot_path), full_page=True)
            assert any(
                bool(sample.get("running"))
                and (
                    int(sample["headingCount"]) > 0
                    or int(sample["listItemCount"]) > 0
                )
                for sample in stream_timeline["samples"]
            ), stream_timeline
            assert int(stream_timeline["samples"][-1]["headingCount"]) > 0, stream_timeline
            assert int(stream_timeline["samples"][-1]["listItemCount"]) >= 3, stream_timeline

            evidence = page.evaluate(
                """async marker => {
                    const deadline = Date.now() + 15000;
                    let messages = {};
                    let traces = {};
                    let context = {};
                    while (Date.now() < deadline) {
                        const conversations = await (
                            await fetch('/api/chat/conversations')
                        ).json();
                        const conversation = (conversations.conversations || [])[0] || {};
                        const id = conversation.conversationId || conversation.id;
                        if (!id) {
                            await new Promise(resolve => setTimeout(resolve, 100));
                            continue;
                        }
                        messages = await (
                            await fetch(`/api/chat/conversations/${encodeURIComponent(id)}/messages`)
                        ).json();
                        traces = await (
                            await fetch(`/api/chat/conversations/${encodeURIComponent(id)}/traces`)
                        ).json();
                        context = await (
                            await fetch(`/api/chat/conversations/${encodeURIComponent(id)}/context`)
                        ).json();
                        const complete = (traces.traces || []).some(trace =>
                            (trace.events || []).some(event => event.kind === 'run.completed')
                        );
                        const persisted = (messages.messages || []).some(message =>
                            message.role === 'assistant'
                            && String(message.content || '').includes(marker)
                        );
                        if (complete && persisted) break;
                        await new Promise(resolve => setTimeout(resolve, 100));
                    }
                    return { messages, traces, context };
                }""",
                marker,
            )
            traces = evidence["traces"].get("traces", [])
            assert len(traces) == 1
            events = traces[0].get("events", [])
            assert any(
                event.get("kind") == "tool.started"
                and event.get("payload", {}).get("tool_name") == "read_file"
                for event in events
            ), events
            assert any(event.get("kind") == "run.completed" for event in events)
            assert any(event.get("kind") == "assistant.delta" for event in events)
            assert any(
                message.get("role") == "assistant"
                and marker in str(message.get("content", ""))
                for message in evidence["messages"].get("messages", [])
            )
            context = evidence["context"]
            used_tokens = (
                context.get("contextUsage", {}).get("usedTokens")
                or context.get("usedTokens")
                or context.get("totalTokens")
                or 0
            )
            assert used_tokens > 0, context
            assert fault_proxy.original_tool_name is not None
            assert fault_proxy.request_count >= 3
            primary_accessibility_path = tmp_path / "web-primary-success-accessibility.json"
            page.screenshot(path=str(primary_screenshot_path), full_page=True)
            primary_accessibility_path.write_text(json.dumps({
                "text": page.locator("#chatMessages").inner_text(),
                "marker": marker,
            }, ensure_ascii=False, indent=2), encoding="utf-8")

            page.wait_for_function("() => !isGeneratingResponse", timeout=15_000)
            previous_assistant_count = page.locator(".chat-msg.msg-assistant").count()
            page.locator(".chat-textarea").fill(
                "And the value after the colon on its second line? Reply with only that value."
            )
            page.locator("#sendBtn").click()
            expect(page.locator(".chat-msg.msg-assistant")).to_have_count(
                previous_assistant_count + 1,
                timeout=2_000,
            )
            follow_up = page.locator(".chat-msg.msg-assistant").last.locator(
                '[data-role="text"]'
            )
            expect(follow_up).to_contain_text(marker, timeout=120_000)
            follow_up_text = follow_up.inner_text().strip("` \n。.")
            assert follow_up_text == marker

            def save_organize_skill(content: str, enabled: bool) -> None:
                page.evaluate(
                    """async ({ content, enabled }) => {
                        const response = await fetch('/api/agent/settings', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                skills: [{
                                    id: 'organize-files',
                                    source: 'built_in',
                                    built_in: true,
                                    enabled,
                                    removed: false,
                                    content,
                                }],
                            }),
                        });
                        if (!response.ok) throw new Error(await response.text());
                        return await response.json();
                    }""",
                    {"content": content, "enabled": enabled},
                )

            def run_skill_turn(prompt_text: str, expected_text: str | None) -> tuple[str, list[dict]]:
                page.wait_for_function("() => !isGeneratingResponse", timeout=15_000)
                conversation_id = page.evaluate(
                    """async () => {
                        await newConversation();
                        return currentConvId;
                    }"""
                )
                assert conversation_id
                previous_count = page.locator(".chat-msg.msg-assistant").count()
                page.locator(".chat-textarea").fill(prompt_text)
                page.locator("#sendBtn").click()
                expect(page.locator(".chat-msg.msg-assistant")).to_have_count(
                    previous_count + 1,
                    timeout=2_000,
                )
                active_assistant = page.locator(".chat-msg.msg-assistant").last
                answer = active_assistant.locator('[data-role="text"]')
                if expected_text:
                    expect(answer).to_contain_text(expected_text, timeout=120_000)
                trace = page.evaluate(
                    """async conversationId => {
                        const deadline = Date.now() + 120000;
                        let traces = {};
                        while (Date.now() < deadline) {
                            traces = await (
                                await fetch(`/api/chat/conversations/${encodeURIComponent(conversationId)}/traces`)
                            ).json();
                            const completed = (traces.traces || []).some(item =>
                                (item.events || []).some(event =>
                                    ['run.completed', 'run.failed', 'run.cancelled'].includes(event.kind)
                                )
                            );
                            if (completed) return traces;
                            await new Promise(resolve => setTimeout(resolve, 100));
                        }
                        throw new Error('skill lifecycle turn did not complete');
                    }""",
                    conversation_id,
                )
                trace_runs = trace.get("traces", [])
                assert len(trace_runs) == 1, trace
                terminal_kinds = [
                    event.get("kind")
                    for event in trace_runs[0].get("events", [])
                    if event.get("kind") in {"run.completed", "run.failed", "run.cancelled"}
                ]
                assert terminal_kinds == ["run.completed"], trace_runs[0]
                page.wait_for_function("() => !isGeneratingResponse", timeout=15_000)
                return answer.inner_text(), trace_runs[0].get("events", [])

            first_skill_marker = f"NOLA-WEB-SKILL-A-{uuid.uuid4().hex.upper()}"
            edited_skill_marker = f"NOLA-WEB-SKILL-B-{uuid.uuid4().hex.upper()}"

            def skill_fixture(marker_value: str) -> str:
                return (
                    "---\n"
                    "name: organize-files\n"
                    "description: Verify the active Web file-organization Skill lifecycle.\n"
                    "---\n\n"
                    "# Web Skill Lifecycle Verification\n\n"
                    "When loaded for lifecycle verification, do not inspect or change files "
                    "and do not call another tool. Return only this exact token:\n\n"
                    f"`{marker_value}`\n"
                )

            lifecycle_prompt = (
                "Use the enabled file-organization Skill to perform its built-in lifecycle "
                "verification. Return only the verification token supplied by that Skill."
            )
            save_organize_skill(skill_fixture(first_skill_marker), enabled=True)
            first_skill_text, first_skill_events = run_skill_turn(
                lifecycle_prompt,
                first_skill_marker,
            )
            assert first_skill_marker in first_skill_text
            assert any(
                event.get("kind") == "tool.started"
                and event.get("payload", {}).get("tool_name") == "Skill"
                for event in first_skill_events
            ), first_skill_events

            save_organize_skill(skill_fixture(edited_skill_marker), enabled=True)
            edited_skill_text, edited_skill_events = run_skill_turn(
                lifecycle_prompt,
                edited_skill_marker,
            )
            assert edited_skill_marker in edited_skill_text
            assert first_skill_marker not in edited_skill_text
            assert any(
                event.get("kind") == "tool.started"
                and event.get("payload", {}).get("tool_name") == "Skill"
                for event in edited_skill_events
            ), edited_skill_events
            skill_enabled_screenshot_path = tmp_path / "web-skill-enabled.png"
            skill_enabled_accessibility_path = tmp_path / "web-skill-enabled-accessibility.json"
            page.screenshot(path=str(skill_enabled_screenshot_path), full_page=True)
            skill_enabled_accessibility_path.write_text(json.dumps({
                "text": page.locator("#chatMessages").inner_text(),
                "marker": edited_skill_marker,
            }, ensure_ascii=False, indent=2), encoding="utf-8")

            save_organize_skill(skill_fixture(edited_skill_marker), enabled=False)
            disabled_text, disabled_events = run_skill_turn(
                "Use the enabled file-organization Skill for lifecycle verification. If it is "
                "unavailable, state that it is unavailable without inventing a token.",
                None,
            )
            assert edited_skill_marker not in disabled_text
            disabled_skill_started = any(
                event.get("kind") == "tool.started"
                and event.get("payload", {}).get("tool_name") == "Skill"
                for event in disabled_events
            )
            disabled_skill_succeeded = any(
                event.get("kind") == "tool.completed"
                and event.get("status") == "succeeded"
                and event.get("payload", {}).get("tool_name") == "Skill"
                for event in disabled_events
            )
            # Skill remains a generic capability while other Skills are enabled,
            # so a model may probe a disabled ID. The resource boundary, not the
            # model's attempted call, is authoritative: disabled content must not
            # load successfully or leak its marker.
            assert not disabled_skill_succeeded, disabled_events
            conversation_metadata = page.evaluate(
                """async () => {
                    const response = await fetch('/api/chat/conversations');
                    if (!response.ok) throw new Error(await response.text());
                    return (await response.json()).conversations || [];
                }"""
            )
            assert conversation_metadata
            assert all(
                item.get("titleSource") == "first_message"
                for item in conversation_metadata
            ), conversation_metadata
            assert all(
                str(item.get("title", "")).strip() not in {"新对话", "New conversation"}
                for item in conversation_metadata
            ), conversation_metadata
            assert not page_errors
            disabled_screenshot_path = tmp_path / "web-skill-disabled.png"
            disabled_accessibility_path = tmp_path / "web-skill-disabled-accessibility.json"
            event_path = tmp_path / "web-agent-events.json"
            persistence_path = tmp_path / "web-persistence-audit.json"
            timeline_path = tmp_path / "web-stream-timeline.json"
            transcript_path = tmp_path / "web-visible-transcript.json"
            page.screenshot(path=str(disabled_screenshot_path), full_page=True)
            disabled_accessibility_path.write_text(json.dumps(page.locator("#chatMessages").evaluate(
                """root => ({
                    role: root.getAttribute('role'),
                    label: root.getAttribute('aria-label'),
                    text: root.innerText,
                    nodes: [...root.querySelectorAll('[role], [aria-label], button, input, textarea')].map(node => ({
                        tag: node.tagName,
                        role: node.getAttribute('role'),
                        label: node.getAttribute('aria-label'),
                        text: node.innerText || node.value || '',
                    })),
                })"""
            ), ensure_ascii=False, indent=2), encoding="utf-8")
            event_path.write_text(json.dumps({
                "initial": events,
                "skillEnabled": first_skill_events,
                "skillEdited": edited_skill_events,
                "skillDisabled": disabled_events,
            }, ensure_ascii=False, indent=2), encoding="utf-8")
            persistence_assertions = {
                "assistant_message_persisted": any(
                    message.get("role") == "assistant"
                    and marker in str(message.get("content", ""))
                    for message in evidence["messages"].get("messages", [])
                ),
                "tool_trace_persisted": any(
                    event.get("kind") == "tool.started"
                    for event in events
                ),
                "conversation_context_persisted": used_tokens > 0,
                "follow_up_reads_persisted_turn": follow_up_text == marker,
                "canonical_conversation_titles_persisted": all(
                    item.get("titleSource") == "first_message"
                    for item in conversation_metadata
                ),
            }
            persistence_path.write_text(json.dumps({
                "passed": all(persistence_assertions.values()),
                "assertions": persistence_assertions,
                "messages": evidence["messages"],
                "traces": evidence["traces"],
                "context": evidence["context"],
            }, ensure_ascii=False, indent=2), encoding="utf-8")
            timeline_path.write_text(
                json.dumps([
                    *[
                        {
                            "stage": "primary-turn",
                            "running": True,
                            "assistantCharacters": int(sample["textLength"]),
                            "headingCount": int(sample["headingCount"]),
                            "listItemCount": int(sample["listItemCount"]),
                        }
                        for sample in stream_timeline["samples"]
                    ],
                    {
                        "stage": "primary-turn",
                        "running": False,
                        "assistantCharacters": len(stream_timeline["finalText"]),
                    },
                ], ensure_ascii=False, indent=2),
                encoding="utf-8",
            )
            transcript_path.write_text(json.dumps(page.evaluate(
                """() => [...document.querySelectorAll('#chatMessages .chat-msg')].map(node => ({
                    role: node.dataset.messageRole || '',
                    text: node.querySelector('[data-role="text"]')?.innerText
                        || node.querySelector('.message-content')?.innerText
                        || node.innerText,
                }))"""
            ), ensure_ascii=False, indent=2), encoding="utf-8")
            record_step(
                "web-ui-tool-skill-persistence",
                platforms=["web"],
                modules=["chat", "agent", "core", "provider", "tools", "skills", "persistence", "ui"],
                boundaries=["web-ui", "http", "agent-host", "provider", "persistence"],
                assertions={
                    "assistant_rendered": any(
                        message.get("role") == "assistant"
                        and marker in str(message.get("content", ""))
                        for message in evidence["messages"].get("messages", [])
                    ),
                    "assistant_streamed_progressively": len(set(positive_lengths)) >= 3,
                    "assistant_bubble_stable": len(stream_timeline["bubbleIds"]) == 1,
                    "markdown_rendered_during_stream": any(
                        int(sample["headingCount"]) > 0 and int(sample["listItemCount"]) >= 3
                        for sample in stream_timeline["samples"]
                    ),
                    "tool_trace_persisted": any(event.get("kind") == "tool.started" for event in events),
                    "context_measured": used_tokens > 0,
                    "unknown_tool_recovered": fault_proxy.original_tool_name is not None,
                    "follow_up_grounded": follow_up_text == marker,
                    "skill_enable_edit_disable": edited_skill_marker in edited_skill_text and edited_skill_marker not in disabled_text,
                    "disabled_skill_resource_rejected": not disabled_skill_succeeded,
                    "disabled_skill_probe_recoverable": not disabled_skill_started or not disabled_skill_succeeded,
                    "canonical_conversation_titles": all(
                        item.get("titleSource") == "first_message"
                        for item in conversation_metadata
                    ),
                    "no_page_errors": not page_errors,
                },
                assertion_evidence={
                    "assistant_rendered": ["web-primary-screenshot", "web-primary-accessibility", "web-persistence"],
                    "assistant_streamed_progressively": ["web-stream-timeline"],
                    "assistant_bubble_stable": ["web-stream-timeline", "web-primary-screenshot"],
                    "markdown_rendered_during_stream": ["web-stream-timeline", "web-primary-screenshot"],
                    "tool_trace_persisted": ["web-events", "web-persistence"],
                    "context_measured": ["web-persistence"],
                    "unknown_tool_recovered": ["web-events", "provider-traffic"],
                    "follow_up_grounded": ["web-persistence", "web-primary-accessibility"],
                    "skill_enable_edit_disable": ["web-events", "web-skill-enabled-screenshot", "web-skill-disabled-screenshot"],
                    "disabled_skill_resource_rejected": ["web-events", "web-skill-disabled-screenshot"],
                    "disabled_skill_probe_recoverable": ["web-events", "web-skill-disabled-accessibility"],
                    "canonical_conversation_titles": ["web-skill-enabled-screenshot", "web-persistence"],
                    "no_page_errors": ["web-primary-screenshot", "web-skill-enabled-screenshot", "web-skill-disabled-screenshot"],
                },
                observations=[
                    {"id": "provider-traffic", "kind": "provider_trace", "path": provider_trace_path},
                    {"id": "web-events", "kind": "event_stream", "path": event_path},
                    {"id": "web-persistence", "kind": "persistence_audit", "path": persistence_path},
                    {"id": "web-primary-screenshot", "kind": "ui_screenshot", "path": primary_screenshot_path},
                    {"id": "web-primary-accessibility", "kind": "accessibility_tree", "path": primary_accessibility_path},
                    {"id": "web-skill-enabled-screenshot", "kind": "ui_screenshot", "path": skill_enabled_screenshot_path},
                    {"id": "web-skill-enabled-accessibility", "kind": "accessibility_tree", "path": skill_enabled_accessibility_path},
                    {"id": "web-skill-disabled-screenshot", "kind": "ui_screenshot", "path": disabled_screenshot_path},
                    {"id": "web-skill-disabled-accessibility", "kind": "accessibility_tree", "path": disabled_accessibility_path},
                    {"id": "web-stream-timeline", "kind": "ui_timeline", "path": timeline_path},
                    {"id": "web-visible-transcript", "kind": "transcript", "path": transcript_path},
                ],
                metrics={"providerRequests": fault_proxy.request_count, "usedTokens": used_tokens},
            )
            step_recorded = True
            browser.close()
    finally:
        if not step_recorded:
            failure_screenshot = tmp_path / "web-chat-failed.png"
            failure_accessibility = tmp_path / "web-chat-failed-accessibility.json"
            failure_events = tmp_path / "web-agent-failed-events.json"
            failure_persistence = tmp_path / "web-failed-persistence-audit.json"
            failure_timeline = tmp_path / "web-failed-stream-timeline.json"
            failure_transcript = tmp_path / "web-failed-visible-transcript.json"
            failure_database = tmp_path / "nola.sqlite"
            page_text = ""
            timeline = None
            transcript = []
            if page is not None and not page.is_closed():
                try:
                    page.screenshot(path=str(failure_screenshot), full_page=True)
                    page_text = page.locator("#chatMessages").inner_text()
                    timeline = page.evaluate(
                        """() => {
                            const value = window.__nolaE2EStreamTimeline;
                            if (!value) return null;
                            const { observer, ...serializable } = value;
                            return serializable;
                        }"""
                    )
                    transcript = page.evaluate(
                        """() => [...document.querySelectorAll('#chatMessages .chat-msg')].map(node => ({
                            role: node.dataset.messageRole || '',
                            text: node.querySelector('[data-role="text"]')?.innerText
                                || node.querySelector('.message-content')?.innerText
                                || node.innerText,
                        }))"""
                    )
                except Exception as evidence_error:
                    page_text = f"Failure evidence collection error: {evidence_error}"
            failure_accessibility.write_text(
                json.dumps({"text": page_text, "failed": True}, ensure_ascii=False, indent=2),
                encoding="utf-8",
            )
            failure_events.write_text(
                json.dumps({"events": locals().get("events", []), "failed": True}, ensure_ascii=False, indent=2),
                encoding="utf-8",
            )
            failure_persistence.write_text(
                json.dumps({
                    "failed": True,
                    "database": str(failure_database),
                    "databaseExists": failure_database.is_file(),
                    "databaseBytes": failure_database.stat().st_size if failure_database.is_file() else 0,
                }, ensure_ascii=False, indent=2),
                encoding="utf-8",
            )
            failure_timeline.write_text(
                json.dumps(timeline or [{"stage": "failed", "running": True}], ensure_ascii=False, indent=2),
                encoding="utf-8",
            )
            failure_transcript.write_text(
                json.dumps(transcript, ensure_ascii=False, indent=2),
                encoding="utf-8",
            )
            failure_observations = [
                {"id": "provider-traffic", "kind": "provider_trace", "path": provider_trace_path},
                {"id": "web-events", "kind": "event_stream", "path": failure_events},
                {"id": "web-persistence", "kind": "persistence_audit", "path": failure_persistence},
                {"id": "web-accessibility", "kind": "accessibility_tree", "path": failure_accessibility},
                {"id": "web-stream-timeline", "kind": "ui_timeline", "path": failure_timeline},
                {"id": "web-visible-transcript", "kind": "transcript", "path": failure_transcript},
            ]
            if failure_screenshot.is_file():
                failure_observations.append(
                    {"id": "web-screenshot", "kind": "ui_screenshot", "path": failure_screenshot}
                )
            record_step(
                "web-ui-tool-skill-persistence",
                platforms=["web"],
                modules=["chat", "agent", "core", "provider", "tools", "skills", "persistence", "ui"],
                boundaries=["web-ui", "http", "agent-host", "provider", "persistence"],
                assertions={
                    "assistant_rendered": False,
                    "unknown_tool_recovered": False,
                    "skill_enable_edit_disable": False,
                },
                assertion_evidence={
                    "assistant_rendered": ["web-events", "web-visible-transcript"],
                    "unknown_tool_recovered": ["provider-traffic", "web-events"],
                    "skill_enable_edit_disable": ["web-events", "web-persistence"],
                },
                observations=failure_observations,
                metrics={"providerRequests": fault_proxy.request_count},
            )
        if browser is not None:
            try:
                browser.close()
            except Exception:
                pass
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
        server_log.close()
        fault_proxy.close()


def test_real_llm_web_cancels_running_tool_and_recovers_next_turn(tmp_path: Path) -> None:
    config_path, saved_config = require_real_llm_configuration()
    if os.environ.get("NOLA_SKIP_REAL_WEB_BUILD") != "1":
        subprocess.run(
            ["cargo", "build", "--quiet", "--bin", "nola"],
            cwd=CLI_ROOT,
            check=True,
            timeout=300,
        )
    assert NOLA_BINARY.is_file(), f"Nola Web binary is missing: {NOLA_BINARY}"

    home = tmp_path / "home"
    home.mkdir()
    provider_trace_path = tmp_path / "provider-cancellation.ndjson"
    provider_proxy = UnknownToolFaultProxy(
        str(saved_config["llm"]["endpoint"]),
        provider_trace_path,
        corrupt_first_tool=False,
    )
    provider_proxy.start()
    install_cli_configuration(config_path, saved_config, home, provider_proxy.endpoint)
    web_port = unused_local_port()
    env = os.environ.copy()
    env["HOME"] = str(home)
    env["NOLA_DB_PATH"] = str(tmp_path / "nola.sqlite")
    server_log = (tmp_path / "web-host-cancellation.log").open("w", encoding="utf-8")
    process = subprocess.Popen(
        [
            str(NOLA_BINARY),
            "web",
            "--host",
            "127.0.0.1",
            "--port",
            str(web_port),
            "--web-root",
            str(WEB_ROOT),
        ],
        cwd=CLI_ROOT,
        env=env,
        stdout=server_log,
        stderr=subprocess.STDOUT,
        text=True,
    )
    base_url = f"http://127.0.0.1:{web_port}"
    try:
        wait_for_http(f"{base_url}/login")
        token_path = web_access_token_path(home)
        deadline = time.monotonic() + 5
        while not token_path.is_file() and time.monotonic() < deadline:
            time.sleep(0.05)
        token = token_path.read_text(encoding="utf-8").strip()

        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(headless=True)
            page = browser.new_page()
            page_errors: list[str] = []
            page.on("pageerror", lambda error: page_errors.append(str(error)))
            page.goto(f"{base_url}/login?next=%2Fchat")
            page.locator("#hostToken").fill(token)
            page.locator('button[type="submit"]').click()
            page.wait_for_url(f"{base_url}/chat", wait_until="domcontentloaded")
            expect(page.locator("#chatMessages")).to_be_visible(timeout=10_000)
            page.evaluate(
                """() => {
                    localStorage.setItem('nola.onboarding.web.v2', 'complete');
                    const guide = document.getElementById('nolaGuide');
                    if (guide) guide.hidden = true;
                    document.body.style.overflow = '';
                }"""
            )
            page.evaluate(
                """async outputDirectory => {
                    const response = await fetch('/api/agent/settings', {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({
                            enable_local_dev_tools: true,
                            host_tools_enabled: true,
                            allowed_dirs: [outputDirectory],
                            agent_mode_enabled: true,
                        }),
                    });
                    if (!response.ok) throw new Error(await response.text());
                }""",
                str(tmp_path),
            )
            page.reload(wait_until="domcontentloaded")
            page.wait_for_function("() => chatAgentModeEnabled === true", timeout=10_000)
            page.locator("#chatToolsMenu > summary").click()
            agent_badge = page.locator("#agentModeBadge")
            expect(agent_badge).to_contain_text(re.compile("开启|On"))
            expect(page.locator("#inspectorModeText")).to_have_text("Agent")

            page.locator(".chat-textarea").fill(
                "Use the bash tool to run `sleep 30` in the foreground. "
                "Wait for it to finish before answering."
            )
            started_at = time.monotonic()
            page.locator("#sendBtn").click()
            conversation_id = page.evaluate(
                """async () => {
                    const deadline = Date.now() + 120000;
                    while (Date.now() < deadline) {
                        const id = activeChatConversationId || currentConvId;
                        if (id) {
                            const response = await fetch(
                                `/api/chat/conversations/${encodeURIComponent(id)}/traces`
                            );
                            const payload = await response.json();
                            const started = (payload.traces || []).some(trace =>
                                (trace.events || []).some(event =>
                                    event.kind === 'tool.started'
                                    && event.payload?.tool_name === 'bash'
                                )
                            );
                            if (started) return id;
                        }
                        await new Promise(resolve => setTimeout(resolve, 100));
                    }
                    throw new Error('real model did not start the long-running bash tool');
                }"""
            )
            page.evaluate("stopActiveGeneration()")
            cancelled_trace = page.evaluate(
                """async conversationId => {
                    const deadline = Date.now() + 15000;
                    while (Date.now() < deadline) {
                        const response = await fetch(
                            `/api/chat/conversations/${encodeURIComponent(conversationId)}/traces`
                        );
                        const payload = await response.json();
                        const run = (payload.traces || [])[0];
                        const events = run?.events || [];
                        if (events.some(event => event.kind === 'run.cancelled')) return events;
                        await new Promise(resolve => setTimeout(resolve, 100));
                    }
                    throw new Error('cancelled terminal event was not persisted');
                }""",
                conversation_id,
            )
            terminal_kinds = [
                event.get("kind")
                for event in cancelled_trace
                if event.get("kind") in {"run.completed", "run.cancelled", "run.failed"}
            ]
            assert terminal_kinds == ["run.cancelled"], cancelled_trace
            assert time.monotonic() - started_at < 15

            page.wait_for_function("() => !isGeneratingResponse", timeout=15_000)
            expect(page.locator("#chat-error-msg")).to_have_text("")
            cancelled_text = page.locator("#chatMessages").inner_text()
            assert "已取消生成" in cancelled_text or "Generation cancelled" in cancelled_text
            assert "无内容返回" not in cancelled_text
            assert "No content returned" not in cancelled_text
            cancelled_tool_card = page.locator(
                '.chat-msg.msg-assistant [data-role="tool-activity-card"]'
            ).last
            expect(cancelled_tool_card.locator('[data-role="tool-activity-status"]')).to_contain_text(
                re.compile("已中止|cancelled")
            )
            assert cancelled_tool_card.locator('.tool-activity-row.cancelled').count() == 1
            assert cancelled_tool_card.locator('.tool-activity-row.error').count() == 0
            cancelled_screenshot_path = tmp_path / "web-cancelled.png"
            cancelled_accessibility_path = tmp_path / "web-cancelled-accessibility.json"
            page.screenshot(path=str(cancelled_screenshot_path), full_page=True)
            cancelled_accessibility_path.write_text(json.dumps(
                page.locator("#chatMessages").evaluate(
                    "root => ({text: root.innerText, labels: [...root.querySelectorAll('[aria-label]')].map(node => node.getAttribute('aria-label'))})"
                ),
                ensure_ascii=False,
                indent=2,
            ), encoding="utf-8")

            recovery_marker = f"NOLA-WEB-CANCEL-RECOVERY-{uuid.uuid4().hex.upper()}"
            page.locator(".chat-textarea").fill(
                f"Reply with only this exact token: {recovery_marker}"
            )
            page.locator("#sendBtn").click()
            recovery_answer = page.locator(".chat-msg.msg-assistant").last.locator(
                '[data-role="text"]'
            )
            expect(recovery_answer).to_contain_text(recovery_marker, timeout=120_000)
            page.wait_for_function("() => !isGeneratingResponse", timeout=15_000)
            assert recovery_answer.inner_text().strip("` \n") == recovery_marker
            expect(page.locator("#chat-error-msg")).to_have_text("")
            assert page.locator(".chat-msg.msg-assistant").last.locator(
                '[data-role="tool-activity-card"]'
            ).count() == 0
            toolbar_layout = page.evaluate(
                """() => {
                    const left = document.querySelector('.toolbar-left')?.getBoundingClientRect();
                    const heading = document.querySelector('.chat-heading')?.getBoundingClientRect();
                    const visibleChildren = [...document.querySelectorAll('.toolbar-left > *')]
                        .filter(node => getComputedStyle(node).display !== 'none')
                        .map(node => node.getBoundingClientRect());
                    return {
                        leftRight: left?.right || 0,
                        headingLeft: heading?.left || 0,
                        childRight: Math.max(0, ...visibleChildren.map(rect => rect.right)),
                    };
                }"""
            )
            assert toolbar_layout["childRight"] <= toolbar_layout["leftRight"] + 1
            assert toolbar_layout["leftRight"] <= toolbar_layout["headingLeft"] + 1
            assert not page_errors
            recovery_screenshot_path = tmp_path / "web-recovered.png"
            recovery_accessibility_path = tmp_path / "web-recovered-accessibility.json"
            event_path = tmp_path / "web-cancellation-events.json"
            persistence_path = tmp_path / "web-cancellation-persistence.json"
            page.screenshot(path=str(recovery_screenshot_path), full_page=True)
            recovery_accessibility_path.write_text(json.dumps(page.locator("#chatMessages").evaluate(
                "root => ({text: root.innerText, labels: [...root.querySelectorAll('[aria-label]')].map(node => node.getAttribute('aria-label'))})"
            ), ensure_ascii=False, indent=2), encoding="utf-8")
            event_path.write_text(json.dumps(cancelled_trace, ensure_ascii=False, indent=2), encoding="utf-8")
            cancellation_persistence_assertions = {
                "conversation_has_stable_identity": bool(conversation_id),
                "cancelled_terminal_persisted": terminal_kinds == ["run.cancelled"],
                "recovery_turn_completed": recovery_answer.inner_text().strip("` \n") == recovery_marker,
            }
            persistence_path.write_text(json.dumps({
                "passed": all(cancellation_persistence_assertions.values()),
                "assertions": cancellation_persistence_assertions,
                "conversationId": conversation_id,
                "terminalKinds": terminal_kinds,
                "recoveryAnswer": recovery_answer.inner_text(),
            }, ensure_ascii=False, indent=2), encoding="utf-8")
            record_step(
                "web-ui-cancellation-recovery",
                platforms=["web"],
                modules=["chat", "agent", "core", "provider", "tools", "cancellation", "persistence", "ui"],
                boundaries=["web-ui", "http", "agent-host", "provider", "persistence"],
                assertions={
                    "cancelled_terminal_persisted": terminal_kinds == ["run.cancelled"],
                    "cancelled_promptly": time.monotonic() - started_at < 15,
                    "next_turn_recovered": recovery_answer.inner_text().strip("` \n") == recovery_marker,
                    "no_page_errors": not page_errors,
                },
                assertion_evidence={
                    "cancelled_terminal_persisted": ["cancel-events", "cancel-persistence"],
                    "cancelled_promptly": ["cancel-events"],
                    "next_turn_recovered": ["cancel-persistence", "recovery-accessibility"],
                    "no_page_errors": ["cancelled-screenshot", "recovery-screenshot"],
                },
                observations=[
                    {"id": "cancel-provider-traffic", "kind": "provider_trace", "path": provider_trace_path},
                    {"id": "cancel-events", "kind": "event_stream", "path": event_path},
                    {"id": "cancel-persistence", "kind": "persistence_audit", "path": persistence_path},
                    {"id": "cancelled-screenshot", "kind": "ui_screenshot", "path": cancelled_screenshot_path},
                    {"id": "cancelled-accessibility", "kind": "accessibility_tree", "path": cancelled_accessibility_path},
                    {"id": "recovery-screenshot", "kind": "ui_screenshot", "path": recovery_screenshot_path},
                    {"id": "recovery-accessibility", "kind": "accessibility_tree", "path": recovery_accessibility_path},
                ],
            )
            browser.close()
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
        server_log.close()
        provider_proxy.close()
