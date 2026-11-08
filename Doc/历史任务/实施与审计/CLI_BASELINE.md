> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# CLI 整体回归基线：未通过

日期：2026-10-11。对应 VERIFICATION，而非完成报告。
执行使用 mbx、新建临时 NOLA_HOME；未加载用户配置或操作生产数据库。

## 最新复测：Task 查询边界整改后

2026-10-11，同一全量命令和串行/16 MiB 测试栈，新的隔离 NOLA_HOME：
**1615 passed、11 failed、4 ignored，exit 101，229.99 秒**。
产物仍为 `target/debug/deps/nola-4b8becf05291a6a7`，本次 SHA-256 为
`e8ed5e1f7822e548e77e8d5e623bee8a1d3551c35430123ebfd8c697d55fe089`。
下面原始 15 失败列表是历史基线，不覆盖或删除它。

原列表此次未失败的五项：retained_mcp_process_mutation_is_transactional_and_replayed_without_second_call、
external_executor_waits_for_the_same_fair_slot_as_a_shell_task、killed_host_is_cold_recovered_by_replay_safety、
another_session_cannot_observe_control_or_consume_a_persisted_agent_task、
single_terminal_result_primitive_preserves_independent_ui_delivery。
不把五项全部归因同一修复：路径身份与读查询整改有直接证据，MCP 差异仍需归因。
原列表其余十项仍失败，错误类别与下表一致。

另新增完整套件失败：
`task_runtime::permission_wait_tests::cold_permission_successor_recovers_without_parent_lease_projection`
在 task_permission_wait_tests.rs 的最终 worker decision 接收处报
`application_rejected: runtime task lease has expired`。该 fixture 设置 1000ms lease；
同产物 task_runtime:: 单独 38 项通过，不能据此判生产实现正确或仅为慢机器。
需区分恢复准入、worker adoption、续租和测试时钟，不允许先延长超时掩盖原因。

用该产物 `--ignored --list` 确认四项未默认执行：

- replica_sync::tests::isolated_worker_chunked_body_survives_http_and_database_restarts
- replica_sync::tests::isolated_worker_round_trip_uses_real_cli_transport
- session::task_result_delivery_tests::task_admission_service_process_fixture
- tests::recorded_long_context_cli_replay_baseline

这里只列明身份，未声称均为缺环境；有些可能由父测试显式调用，需分别核对调用链与本次子进程证据。
默认栈/并发仍未复测，旧 SIGABRT 不因串行通过更多测试而关闭。

## 原始全量运行

```sh
source scripts/lib/rust_build_cache.sh
nola_rust_build_cache_init
export NOLA_HOME=$(mktemp -d /tmp/nola-architecture-cli-serial.XXXXXX)
export RUST_MIN_STACK=16777216
nola_cargo test -p nola_cli --bin nola --quiet -- --test-threads=1
```

结果：exit 101，1611 passed、15 failed、4 ignored，总计 1630；测试阶段 197.63 秒。
产物：`target/debug/deps/nola-4b8becf05291a6a7`，SHA-256
`d78194a03b22b4a989bd52784798fc7933b98b987cc2f0637e748327ae9952df`。
这是 dirty worktree 的当前产物，不代表某个完整 git commit。

默认并发/默认线程栈的第一次运行在
`real_host_permission_interruption_repeated_resume_preserves_original_call` 栈溢出并以 SIGABRT 退出，
未收集完整错误。以上增大测试栈的串行运行仅用于诊断，不证明生产栈安全或并发验收通过。
`compaction_cache_fork_recovers_without_executing_tools_and_accounts_both_attempts` 在并发运行失败，
单独运行和串行整体运行通过；并发差异仍需查明，不能据此忽略失败。

## 可复现失败清单

以下为测试函数名，省略公共模块前缀；入口抽取后用 `nola_cargo test -p nola_cli --lib <name>` 定点运行。
上文 `--bin nola` 是实际旧基线命令，保留为历史证据；它现在不再包含共享模块单元测试，不能用于声称新一轮已覆盖。
现象不是根因结论，不预先认定全为已有问题或全由本轮修改引起。

| 测试 | 实际失败证据 | 相关契约 |
|---|---|---|
| core_task_event_uses_the_shared_lifecycle_contract | 实际 lineage 多 resumed_from_task_id:null，fixture 缺该字段 | MOD-01、Task schema |
| desktop_authorization_allowlist_preserves_deferred_model_exposure | Visible != Deferred | MOD-07/11 |
| retained_mcp_process_mutation_is_transactional_and_replayed_without_second_call | 预期 invocation 数 1，实际 0 | INV-01/09、MOD-11 |
| lost_notification_queue_is_seeded_from_committed_heads_once_per_run | 缺对应 assistant_journal_committed_v1 通知 | CLIENT-02 |
| same_run_subscriber_return_seeds_commits_made_without_a_presenter | 返回订阅前预期通知集合为空 | CLIENT-02 |
| internal_permission_rules_do_not_exempt_private_memory_from_consent | conversation_recall 为 Allow，预期 Ask | AUTH-01、MEM-01 |
| allow_all_picker_updates_typed_core_configuration_and_can_restrict_again | 缺 --endpoint/llm.endpoint | 配置隔离、MOD-08 |
| external_executor_waits_for_the_same_fair_slot_as_a_shell_task | 找不到预期 queued external_executor | MOD-05/10 |
| killed_host_is_cold_recovered_by_replay_safety | running != retry_waiting | REC-01 |
| another_session_cannot_observe_control_or_consume_a_persisted_agent_task | 预期计数 1，实际 0 | Task scope/receipt |
| single_terminal_result_primitive_preserves_independent_ui_delivery | 模型已消费后 UI 可见结果数为 0，预期 1 | Task receipt、CLIENT-02 |
| concise_foundational_tool_descriptions_stay_within_authored_budgets | tool_search 描述预算断言失败 | MOD-11、P08 |
| large_tool_preview_exposes_exact_utf8_continuation_offset | offset > preview.chars.count 断言失败 | MOD-06、UTF-8 reader |
| encrypted_http_reads_exact_committed_journal_and_rejects_forged_identity | expected 一个初始 cursor，实际 heads=[] | CLIENT-03 |
| web_context_estimate_uses_host_exposure_without_removing_deferred_tools | 暴露 names 含 write_file，预期不含 | MOD-08/11 |

四项 ignored 不算通过；需在最终验收逐个识别其运行条件。
下一轮按同一契约聚类：journal 通知/heads、工具暴露/权限、Task 恢复/结果、fixture 与配置。
修复必须解释原断言和实现哪个符合权威契约，不允许批量更新期望值消除红灯。
