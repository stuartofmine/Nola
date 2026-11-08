> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# V2 history reader acceptance evidence

Date: 2026-09-08

This is a partial acceptance record under `ARCHITECTURE_V2.md`, not a replacement
for its cross-platform architecture requirements. The overall refactor is not
complete.

## Implemented boundary

- Application Service remains the reader's transaction boundary. The Mac view
  does not open SQLite or reconstruct canonical history from streamed deltas.
- Oversized text in a history window is returned as a bounded UTF-8 preview with
  explicit conversation/message/Part identity and original byte length.
- `read_conversation_text_range` reads at most 32 KiB per request. Ownership,
  complete text state, offsets, UTF-8 boundaries, and returned lengths are checked.
- The Mac bubble labels deferred content as a preview. A separate original-text
  reader retains one page and at most 64 backward navigation positions.
- The reader displays literal original text rather than independently parsing
  each arbitrary byte page as Markdown. The normal chat renderer is separate.
- Page changes discard old response delivery. Closing the view cancels its task;
  this does not yet prove interruption of an already-blocked transport read.
- Protocol version 11 prevents reuse of an older service that cannot provide
  required deferred-text window metadata.

## Observed evidence

- Protocol and IPC client regression run: 29 tests passed after version 11.
- Mac build-for-testing succeeded after reader and keyboard-navigation changes.
- Native original-text range test passed with real Service-backed storage,
  multilingual UTF-8 reconstruction, and foreign-conversation rejection.
- The last full history suite before adding interaction tests passed 8 tests.
  This is not a claim that the current, expanded suite is all green.
- English dark and Simplified Chinese compact screenshots were inspected after
  adding an opaque system background and matching AppKit/SwiftUI appearance.
- Native keyboard interaction passed forward/forward/back/start actions through
  the actual window's key-equivalent dispatch and verified contiguous original
  byte ranges. Its recorded method duration was 48.720 seconds; the slowdown is
  not yet attributed.
- The corresponding accessibility-press test currently fails to discover the
  SwiftUI controls. It is retained as a failing acceptance gap, not skipped or
  replaced by direct calls to the navigation function.

Artifacts retained from these runs:

```text
/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-mac-history-acceptance-9xwedkje
/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-mac-history-acceptance-gsvupnkg
/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-mac-history-acceptance-lmjyv1d9
```

These paths are local run artifacts, not portable fixtures or committed user data.

## Test execution safety

Run built native tests with `python3 scripts/run_mac_history_acceptance.py`.
The runner isolates state, disables real LLM/keychain/permission tests, retains
results, applies a deadline, and fails on skipped or zero executed tests.

An explicit architecture-qualified Mac destination did not prevent Xcode from
implicitly probing a locked iPhone. The runner now watches its log and terminates
its owned test process group on remote-device connection markers. This prevents
continued polling after detection; it does not guarantee prevention of the first
probe. No attempt is made to unlock devices or change global Xcode preferences.

Guard tests use owned synthetic subprocesses, not physical devices:

```sh
python3 -m unittest discover -s scripts/tests -p test_mac_history_acceptance_runner.py -v
```

Four guard tests passed: detected probe termination, marker across read boundary,
clean exit, and bounded timeout. Per-stage native navigation timing instrumentation
has been added but has not yet produced a measured run.

## Remaining acceptance work

- Find the accessibility discovery failure and verify actual pointer/AX actions.
- Attribute the 48.720-second navigation test to setup, dispatch, reading, layout,
  or teardown using stage timings. A passing result is not a latency acceptance.
- Verify closing/reopening, concurrent readers, rapid navigation, and conversation
  switching while reads are pending, including memory bounds and stale delivery.
- Prove preview metadata cannot enter backups as a complete canonical transcript.
- Complete large canonical import via staged content adoption. Upload staging
  alone does not solve single-message imports that exceed the command frame.
- Bound large structured Parts and live text, not only completed historical text.
- Wire equivalent bounded original-content access into the other clients.
- Complete the remaining single-writer, effect, permission, context, memory,
  scheduling, and cross-platform requirements of `ARCHITECTURE_V2.md` separately.
