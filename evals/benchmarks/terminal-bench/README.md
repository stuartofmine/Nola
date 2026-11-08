> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Nola Terminal-Bench baseline

This directory vendors 20 complete tasks from the official Terminal-Bench
4.0.0 release. The tasks retain their original instructions, environments,
oracle solutions, tests, reviewer notes, canary strings, and attribution.

## Provenance

- Repository: `https://github.com/harbor-framework/terminal-bench.git`
- Release: `v4.0.0`
- Commit: `452bf305c6daa62fc59061d22133a7cbc7c1572e`
- License: Apache-2.0; the unmodified upstream license is in `LICENSE`.
- Release archive checksum: run `git archive --format=tar v4.0.0 | shasum -a 256`
  in the upstream checkout and compare it with `upstream.artifact_sha256` in
  `benchmark.json`.

The official Harbor Hub maps `terminal-bench/terminal-bench` to 66 tasks, and
the official benchmark page identifies recent runs of that dataset as
Terminal-Bench 4.0.0. The Git tag is the immutable source used here; no content
was sampled from the floating `main` branch.

## Selection

The source release has seven top-level task categories. The baseline assigns
2-4 places to every category, then orders each category by the upstream expert
time estimate. It selects evenly spaced ranks, using a seeded SHA-256 tie break.
This produces exactly 20 cases and covers every category plus low, medium, and
high relative expert-effort bands. `materialize.py` is the executable selection
specification; `benchmark.json` records the resulting IDs and checksums.

The labels describe relative expert effort inside this release:

- `low`: at most 2 hours
- `medium`: more than 2 and at most 6 hours
- `high`: more than 6 hours

Terminal-Bench is a frontier benchmark, so `low` does not mean an easy task.

## Reproduce

Check out the pinned upstream commit and regenerate the snapshot:

```sh
git clone --branch v4.0.0 --depth 1 \
  https://github.com/harbor-framework/terminal-bench.git /tmp/terminal-bench-v4
python3 evals/benchmarks/terminal-bench/materialize.py /tmp/terminal-bench-v4
```

Each case hash covers every ordinary file in its task directory. The manifest
also records separate hashes for the descriptor, environment Dockerfile,
test entry point, and oracle entry point. This makes incomplete copies and
silent upstream changes detectable without network access.

The task directories use the native Harbor task format and can be addressed by
their `source_path` values in `benchmark.json`. A complete run needs Harbor and
the container/GPU resources declared by each task's `task.toml`.

Aggregate one or more Harbor job/run directories while keeping the manifest's
fixed denominator:

```sh
python3 scripts/aggregate_terminal_bench.py path/to/job-or-run [...] \
  --json-output report.json --markdown-output report.md
```

The report retains every discovered attempt. It reports Harbor's canonical
`rewards.reward` separately from CTRF assertions: CTRF is diagnostic evidence
and may describe a single scorer wrapper or prerequisite checks rather than the
task score. The JSON includes lifecycle stages, invalid infrastructure runs,
agent failures, verifier failures, declared artifact transport, and individual
assertion identities. It never substitutes another numeric verifier field or an
assertion pass rate for a missing official reward.

Before a paid Terminal-Bench run, `scripts/run_external_benchmark.py` compares
the selected tasks' declared agent and verifier CPU/memory requests with the
active Docker daemon. It writes `preflight.json` and stops before Harbor starts
if a task cannot fit. `--skip-runtime-preflight` is available for externally
managed runtimes whose capacity is not represented by the local Docker daemon.

The Linux runtime passed to Harbor must come from `scripts/build_cli_runtime.sh`.
Code intelligence is compiled into `nola` and executes through the same native
Host capability path used by CLI, Rust Web, macOS, and Windows. The benchmark
adapter uploads no extra Code Memory executable and exposes no benchmark-only
injection option.

The Linux Host requires Python 3, Git, and `ps` (`procps` on Debian images,
`procps-ng` on Fedora). The adapter installs missing Git/process-observation
packages inside the disposable agent environment. It preserves an existing
repository HEAD and creates a baseline only when the task has none. Missing
process observation is not evidence that a command stopped; do not bypass that
check to make a slim image appear functional.

## Performance self-checks during agent work

A local timing result proves only its measured workload. Match public workload
properties (for example positive versus negative decisions), input size and
mutation semantics; keep separate populations separate in the report. Use
fresh independent objects and representative unseen seeds. Apply warmup,
repeat policy and timing boundaries symmetrically to baseline and candidate.
Report per-case ratios and the requested aggregation (a geometric mean is not
an arithmetic mean or a ratio of total times), including variance and weak
cases. Repeated warm calls on one object must not be labelled a cold-input
benchmark. Exclude setup only when it is excluded for both implementations.
A self-test is not an official-gate pass; retain the distinction until an
independent verifier checks the frozen artifact. Never reveal hidden cases
or tune implementations against verifier feedback during a scored run.
