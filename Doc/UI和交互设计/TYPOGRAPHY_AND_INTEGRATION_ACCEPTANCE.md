# Typography and capability integration

## Font decisions

Native Apple clients retain system text styles (SF/PingFang), including Dynamic Type and optical sizing. Android retains its platform typography and CJK fallback. These defaults avoid redistributing Apple-only fonts and preserve accessibility sizing.

Web uses locally bundled Inter 4.001 for supported scripts and the device's CJK fallback for Chinese; code uses locally bundled JetBrains Mono 2.305 with programming ligatures disabled. Both fonts use OFL 1.1; licenses and source URLs ship under `/ui/font-licenses/` in the built Web distribution. Font loading uses `swap` so text remains readable before fonts load, and no external font service is contacted. Total compressed font files are approximately 434 KiB.

Considered Chinese alternative: Source Han Sans / Noto Sans CJK. The inspected Noto Sans SC variable TTF is about 17 MiB; it is not bundled in this change. A future offline Chinese font option needs script coverage and package-size evaluation rather than a small fixed-character subset that breaks arbitrary conversations. Inter and JetBrains Mono do not supply Chinese glyphs.

References: [Apple fonts](https://developer.apple.com/fonts/), [Inter](https://rsms.me/inter/), [Source Han Sans](https://github.com/adobe-fonts/source-han-sans), [JetBrains Mono](https://www.jetbrains.com/lp/mono/).

## Integration boundaries and regressions

UI previews and editor persistence tests do not prove a successful model/tool run. Keep the following evidence separate:

- Shared MCP transport: real child processes and loopback HTTP; cancellation, timeouts, uncertain side effects and no automatic mutation retry.
- Reference interop: official Python MCP SDK 1.30.0; both stdio and stateful Streamable HTTP initialize and discover tools/resources/prompts, execute calls, and reject calls before admission and after revocation. This harness exercises the host admission boundary directly; it does not exercise a client permission dialog or model-selected MCP call.
- Editor persistence: stale revisions, atomic batches, enable/edit/delete behavior and restart receipts.
- Real model Skill runs: isolated CLI/Service databases, canonical sealed run receipt, committed Skill/tool sequence and artifact validation. The selected route comes from current `llm.connections/default_model`; the named credential is resolved only in memory and the source configuration is never rewritten. Explicit test capability policy remains required; model names do not imply capabilities.

The reference SDK exposed a stdio bug: resolving a venv Python symlink changed its invocation context and lost installed packages. The host still executes the frozen resolved image, but on Unix passes the original absolute invocation path as `argv[0]`. Both paths participate in the existing private launch identity; this does not change Nola's public protocol, ABI or database format. No new authority is granted by a path alias.

## Reproduction

```sh
python3 -m venv /tmp/nola-mcp-sdk
/tmp/nola-mcp-sdk/bin/pip install mcp==1.30.0
NOLA_MCP_SDK_PYTHON=/tmp/nola-mcp-sdk/bin/python cargo test -p nola_mcp_host --test sdk_interop -- --ignored
cargo test -p nola_mcp_host --features test-support --lib
cargo test -p nola_application --test mcp_editor_receipt_e2e --test skill_editor_receipt_e2e --test skill_catalog_e2e
PYTHONPATH=scripts python3 -m unittest discover -s scripts -p test_live_skill_e2e.py
PYTHONPATH=scripts/e2e python3 -m unittest discover -s scripts/e2e -p test_current_model.py
python3 scripts/test_web_mcp_baseline_e2e.py
```

Paid Skill acceptance uses `NOLA_RUN_LIVE_SKILL_E2E=1 python3 scripts/live_skill_e2e.py analyze-data review-documents`, current built CLI/Service/oracle binaries, and a route-bound execution declaration. Never put production credentials in commands or commit transcripts or runtime databases.

Validation on 2026-11-10: 17 host tests, 2 reference SDK tests, 20 editor/catalog tests, 22 Skill harness tests and 3 current-route tests passed. Web/Service MCP mutation isolation passed. Two real configured-model Skill runs produced validated analysis and review artifacts and sealed successful runs. Font specimens at 390/1280 px in light/dark loaded both local fonts without horizontal overflow. Mac real-model Skill enable/edit/disable lifecycle also passed (1 test, 0 skipped, 0 failed) using the dedicated Test configuration. These results do not establish native iOS/Android UI acceptance or every third-party MCP server's compatibility.
