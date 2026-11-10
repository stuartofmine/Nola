> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Native Code Memory architecture

Nola implements Code Memory as a Rust library linked into `nola` and
`nola-agent-host`. CLI, Rust Web, macOS, and Windows reach it through the same
Host provider. It has no daemon, MCP transport, stdio bridge, executable
discovery, or companion process. Mobile clients do not register this provider.

This design was informed by a source audit of
[`DeusData/codebase-memory-mcp`](https://github.com/DeusData/codebase-memory-mcp)
tag `v0.10.8`, commit
`46ae198fc11cda80e817acbc5f5908d7c2de7032`. The upstream project is MIT
licensed. Nola reimplements the engine boundary in Rust and does not vendor the
upstream daemon, MCP server, generated grammar tree, vector assets, or C source.

## What the engine is

Code Memory is a persistent code graph, not a synonym for grep. Its pipeline is:

1. Discover supported files within a canonical workspace boundary while
   honoring ignore rules and recording read, size, and parse failures for
   discovered supported files.
2. Detect and parse 67 statically linked languages and engineering formats
   without downloading a grammar at runtime. Nineteen language profiles add
   explicit definition extraction for Bash, C, C#, C++, Dart, Go, Java,
   JavaScript, Kotlin, Lua, PHP, Python, Ruby, Rust, Solidity, Swift, and
   TypeScript/TSX, plus Verilog and SystemVerilog design units, functions,
   tasks, and module instantiations. Every parsed language participates in
   bounded text
   search; graph depth is reported separately from parser availability.
3. Resolve a textual call or import only when its target name identifies one
   graph node. Ambiguous references remain raw evidence and do not become graph
   edges.
4. Publish nodes, edges, file identities, coverage records, and a monotonically
   increasing project generation to SQLite.
5. Serve bounded search, trace, snippet, architecture, coverage, and status
   queries directly through the Host tool boundary.

The graph uses stable project-relative qualified names. Calls carry a
resolution strategy and confidence; consumers can distinguish parser evidence
from a fallback instead of treating every edge as certain. Coverage records
distinguish a complete parse, partial parse, oversized source, and I/O or
extraction failure. Ignored and unsupported files are not enumerated yet, so
coverage explicitly reports `repository_scope_complete=false`. The graph
schema returns the exact grammar registry and semantic-profile registry so a
caller can check this distinction before relying on graph edges.

## Persistence and freshness

Each project records its canonical root, indexed generation, file content hash,
mtime, size, language, and parse coverage. Content identity decides whether a
file changed; mtime is only a discovery optimization. Removed files and their
dependent graph rows are deleted in the same publication transaction.

Readers observe one complete generation. Cancellation or an indexing error
preserves the preceding generation. Before a query, file-set metadata is
compared with the published index. Files actually returned by search, trace,
or snippet operations are then verified by content digest. The Host adds
independent repository/path/symbol provenance before exposing results to the
model.

Incremental indexing reuses committed, strong file identities where available,
hashes remaining candidates, re-extracts changed files, and removes deleted rows.
It re-resolves affected definitions and callers from retained raw references in
one transaction; unrelated edges are preserved. Incompatible schema or extractor
versions are explicitly rejected without deleting or rebuilding existing data.

Discovery does not initialize every linked parser. Arborium selects one grammar
for the detected file, and each indexing worker creates and caches a parser only
when that language first appears in the current project. Statically linked
grammar code affects executable size and link time, but it is not all executed
at application startup. File reads, digests, and parsing run in a bounded pool
of at most eight workers; SQLite publication remains ordered and
single-transactional. Languages without a semantic profile skip AST definition
traversal after parse coverage is known.

## Native tool boundary

Core owns stable `codebase_*` capability identities and effect declarations.
The Host registers them as built-in capabilities and invokes the Rust provider
before generic MCP routing. Read-only operations are concurrent-safe at the
catalog boundary; each engine handle serializes its SQLite connection, while
separate processes use WAL. Workspace roots and cancellation come from the
frozen Host call; the provider cannot widen access from request arguments.

The registered surface contains exactly eleven operations: repository index,
project list, index status, symbol search, text search, call trace, symbol
snippet, architecture summary, graph schema, coverage, and index deletion.
Graph query, changed-impact analysis, architecture decisions, and runtime trace
ingestion are not registered until their native data semantics exist. Direct
internal compatibility calls return a stable unsupported result.

## Deliberate differences from the upstream deployment

Nola does not port upstream process supervision, installer behavior, file
watcher daemon, MCP framing, or shell-based grep. It also omits heuristic
concept-token injection and weak cross-language suffix matching. Those rules
can improve recall on selected corpora while making false graph edges harder to
diagnose. Nola prefers unresolved evidence over a confident wrong connection.

The upstream Cypher subset, typed-language resolvers, semantic vectors,
cross-repository links, clone detection, and clustering are later engine
layers. A feature is exposed only after its data and failure semantics are
implemented; tool compatibility is not claimed from matching a name alone.

## Verification contract

Engine tests use temporary multi-language repositories with natural symbol
names. They cover ignore rules, partial parses, ambiguous calls, imports,
deletions, restart persistence, generations, cancellation, paging, snippet
bounds, stale evidence, symlink swaps, and cache invalidation across database
connections. Host tests exercise native capability registration, workspace
confinement, cancellation, and exact schemas without an MCP server, subprocess,
environment override, or companion artifact.

Performance regressions are represented by stable behavior and query-complexity
tests rather than machine-specific wall-clock assertions:

- `schema::tests::replacement_cascades_use_foreign_key_indexes` verifies with
  SQLite's query planner that file and symbol replacement seek through every
  required foreign-key index and never scan the growing dependent tables.
- `engine::tests::unchanged_generation_does_not_rebuild_edges` verifies that an
  unchanged generation preserves the already-published edge set.
- `engine::tests::symbol_dense_file_can_be_replaced_without_stale_graph_rows`
  replaces a generated 512-symbol call chain and verifies that old nodes and
  edges disappear while the complete new graph is published.
- `engine::tests::nola_repository_release_performance_baseline` is an explicit
  ignored release acceptance test. It records the 5,171 ms cold and 118 ms
  unchanged observations in code, allows a threefold cold-host variance and a
  one-second unchanged ceiling, and prints current file/symbol counts. Run it
  with `cargo test --release -p nola-code-memory
  nola_repository_release_performance_baseline -- --ignored --nocapture`.

The measured repository-scale timings below remain the manual release-build
acceptance baseline. Keeping absolute seconds out of ordinary unit tests avoids
false failures caused by CI load, CPU model, storage, and debug builds.

The programming-language set uses a 0.1% floor calculated from
[GitHub Innovation Graph](https://innovationgraph.github.com/global-metrics/programming-languages)
2026 Q1 developer-language push activity aggregated across reported economies.
Engineering formats such as JSON, YAML, TOML, Markdown,
Dockerfile, CMake, SQL, and Protocol Buffers are retained separately because
the programming-language denominator does not measure their value to repository
analysis. The support policy and current registry are returned by
`get_graph_schema`.

On the Nola repository, the initial 66-grammar release-build smoke run
discovered 3,145 supported files across 28 languages and extracted 77,097
symbols. A phase-instrumented run took 27.28 seconds: 68 ms for discovery, 1.89
seconds for parallel reading, hashing, and parsing, 24.19 seconds for SQLite
publication, 0.90 seconds for edge rebuilding, and 0.23 seconds for commit and
reporting. The database contained 558,713 raw references. This ruled out grammar
initialization and parsing as the dominant bottleneck.

The publication slowdown came from `DELETE FROM references_raw WHERE file_id=?`
running once per indexed file without an index on `file_id`. As the table grew,
each delete scanned the accumulated reference rows, making a cold build approach
quadratic complexity across files. Adding the missing foreign-key lookup index
reduced two formal repetitions of the same cold-index workload to 4.951 and
4.847 seconds, with parsing and publication both near two seconds. An
unchanged incremental run previously spent about 1.03 of its 1.20 seconds
rebuilding an identical edge set; edge rebuilding is now skipped when no file
graph changed.

The same audit found that SQLite foreign-key cascades delete edges by
`source_id` and `target_id`, while the earlier compound indexes began with
`project_id` and could not serve those lookups. Re-indexing one generated file
with 9,204 symbols and 84,952 raw references still exceeded 50 seconds before
the missing foreign-key indexes were added; the same diagnostic workload then
completed in 2.707 seconds. These indexes protect changed-file performance and
avoid a second data-dependent table-scan failure.

The final build with all three foreign-key indexes and the unchanged-edge fast
path indexed the current 3,148-file, 77,119-symbol repository in 5.171 seconds.
The immediately following unchanged run completed in 118 ms. These figures are
local development-machine observations and are retained as a regression
baseline rather than a portable performance guarantee.

The regression is executable as the ignored release test
`engine::tests::nola_repository_release_performance_baseline`. A 2026-09-12
run on the later 67-grammar worktree indexed 3,151 files and 77,236 symbols in
5.435 seconds and completed the unchanged generation in 106 ms, passing the
15.513-second cold-index and 1-second unchanged-generation ceilings. Run it
with `cargo test --release -p nola-code-memory
nola_repository_release_performance_baseline -- --ignored --nocapture`; routine
debug suites skip the repository-scale, host-sensitive measurement.

A final pre-commit run after the benchmark and attachment changes indexed 3,155
files and 77,376 symbols in 6.724 seconds; the unchanged generation completed
in 90 ms. The same executable acceptance case passed both ceilings. This later
measurement is the current release-gate observation; the earlier figures remain
useful for identifying normal worktree and host variance.

The index found `CliAgentHostDriver` through native symbol search and reported
373 partial parses, dominated by vendored macro-heavy C/C++. Removing Common
Lisp also stopped 168 OpenCL `.cl` files from being misclassified.

The 111-grammar build produced a 215 MiB Mach-O executable, or 207 MiB after
local symbol stripping. The 66-grammar build produced 119 MiB, or 111 MiB after
the same stripping step: reductions of 44.8% and 46.6%, respectively. Verilog is
now retained as an explicit EDA exception to the general usage floor, bringing
the registry to 67 grammars. Lazy parser initialization reduces runtime work but
cannot remove linked bytes, so the usage floor primarily reduces desktop
package size while keeping common repository and engineering formats.

The upstream v0.10.8 source uses size-aware dynamic worker scheduling,
whole-database staging, deferred FTS construction, and a specialized direct
SQLite page writer. Nola already uses a bounded dynamic worker queue and one
semantic extraction path. It does not copy the direct page writer or disable
durability on the live database because those mechanisms carry substantially
greater schema and cross-platform risk. The next safe candidates are slow-file
telemetry, size-aware scheduling, moving parse work outside the database lock,
and project-scoped bulk FTS construction; each should be adopted only after a
phase benchmark shows a material gain. Raising SQLite's cache to 64 MiB improved
this workload by only about 2.4%, so Nola keeps the smaller default instead of
adding that per-connection memory cost.

## Autonomous model selection acceptance

`code-memory-autonomy-large-repository` exercises the real CLI Agent Host with
the configured `openrouter/free` model, an isolated application database, the
Nola repository, and a natural cross-file architecture question that does not
name or force a tool. Before the exposure fix, multiple bounded runs used only
generic file tools and never discovered Code Memory. The native provider had
incorrectly inherited the `Deferred` exposure used for third-party MCP tools,
so its schemas were absent from the initial model step.

The native repository workflow now directly exposes `codebase_index`, symbol
and text search, call trace, symbol read, and architecture summary. Management
operations remain available through typed tool discovery. Building the
rebuildable derived index is classified as an investigative Host effect while
remaining serial at the storage boundary; deleting it remains a mutation. The
index schema also uses `project`, matching every follow-up query, while the
engine continues to accept the earlier internal `name` alias. In the post-fix
run,
the model independently attempted `codebase_search`, recovered from the
unindexed-project result with `codebase_index`, then called
`codebase_search`, `codebase_read_symbol`, `codebase_trace`, and
`codebase_text_search`. The full Nola analysis did not publish a final answer
within the 150-second acceptance window because the free routed model continued
issuing overlapping searches; tool selection passed, while bounded completion
remains a separate model/SOP issue.

A final `code-memory-autonomy-permissionless` run removed `--yes` and stopped as
soon as the native workflow was proven. The model called `SOP`, then
`codebase_index`, then three `codebase_search` operations. Indexing completed
with 3,148 files and 77,133 symbols; the first search completed, and no
permission event occurred. This verifies that the model can select and execute
the derived-cache workflow autonomously without a prompt keyword router or a
mutation approval bypass.

`code-memory-autonomy-small-repository` asks the same configured model to trace
a four-file Rust checkout flow. The model chose bounded file listing and reads,
made no Code Memory call, and completed with the correct call chain. This is the
desired selection boundary: Code Memory is available without prompt rewriting,
while a small project does not pay indexing cost when direct evidence is
cheaper.

## 项目规模与初始化前置检查（2026-10-20）

共享 Rust Host 对已经通过 Core 资源授权的仓库分析调用执行初始化前置检查。范围来自工具的具体 `path`/`root` 参数，不从用户语言推断，也不扫描所有可访问目录。普通聊天不触发扫描。

复用索引器的语言识别、ignore 和不跟随符号链接规则，排除 Markdown、JSON、YAML、TOML 等文档/数据配置格式后，统计可识别的代码文件：

- 20 个及以下保持可选，直接执行查询。
- 大于 20 个且范围未初始化：查询返回 `code_memory_initialization_required` 和 `NotStarted`，提供实际数量、精确范围及 `codebase_index` 调用；工具尚未暴露时提供 `tool_search` 发现操作。改变查询词不能绕过前置条件。
- 索引必须走独立的正常 Core 工具调用和授权链。`codebase_index` 声明 `repo_path` 的 `read_tree` 资源；Host 使用索引器实际文件清单核对敏感文件与忽略规则，并在执行前重新校验资源事实。前置检查本身不执行索引、不扩大范围。
- 只有索引成功发布正数 generation 后，才建立对应根目录的进程内已提交状态。在 macOS/Linux 等 Unix Host 上，每次仓库分析前检查当前 generation、文件清单及强文件身份；状态一致则跨回合复用，新增/删除/改写则要求增量刷新。同一回合的外部编辑也会使就绪状态失效。另一项目不能借用该状态；刷新失败不提交新的文件基线。
- 统计失败、取消或 provider 不可用均返回明确失败；删除索引使本轮就绪状态失效。显式工具允许列表排除 `codebase_index` 时，保留该工具限制，不暗中执行索引。

前置检查覆盖 `code_search`、`repo_map`、`symbol_search`、`implementation_search`、`workspace_symbols`、`code_navigation`、`call_graph`、`file_dependencies`、`read_symbol` 的目录范围调用。单文件读取、Shell 及非代码工具不按用户措辞强制改路由；本契约不声称拦截所有可能的文件访问方式。

回归通过真实 Host dispatch 验证阻拦搜索、未授权索引失败、成功索引后搜索执行及不同范围隔离；另有数量临界值、失败/删除就绪状态和实际索引资源权限测试。

真实模型回归已覆盖 21 文件前置阻拦后的索引发现与恢复、20 文件直接查询，以及基于已读源码的自然追问；验证结果见当前任务记录。测试在隔离数据库和源码夹具中执行，不代表真实大型项目的全量性能验收。

## 增量更新与就绪状态优化（2026-10-20）

参考上游 `DeusData/codebase-memory-mcp` 的提交 `4c80d89edf1283cfeaa7e60ecbf40c9d2eb2b689`：

- [增量更新](https://github.com/DeusData/codebase-memory-mcp/blob/4c80d89edf1283cfeaa7e60ecbf40c9d2eb2b689/src/pipeline/pipeline_incremental.c)按文件变化执行局部更新。
- [变更检测](https://github.com/DeusData/codebase-memory-mcp/blob/4c80d89edf1283cfeaa7e60ecbf40c9d2eb2b689/src/watcher/watcher.c)区分持久脏状态与新的变化，并在更新成功后提交检测基线。

Nola 保留 Rust 原生引擎及 Core 权限边界，不引入 MCP 进程或新的后台源码读取。实现如下：

1. 在支持强身份的 Unix 平台，记录 device、inode、size、mtime、ctime；只有与同一已提交 generation 匹配的状态才跳过内容读取。仅 size/mtime 相同不构成可信快路径。进程重启后的第一次刷新仍校验内容哈希。
2. 完全无变化时不启动解析线程、不更新 SQLite generation、不失效查询缓存；只做目录与元数据核对。
3. 收集变化文件修改前后的名字与限定名，重新解析这些定义及引用这些名字的调用方。保留其他关系，处理目标从唯一变歧义、从歧义恢复唯一、目标删除与重命名。关系计算及文件数据在同一事务内发布。
4. 进程内就绪状态跨回合复用，并在每次仓库分析前核对源文件变化。没有后台文件监听；源码变化由下一次访问发现，实际刷新仍是经 Core 授权的 `codebase_index`。
5. 不具备强文件变更身份的平台继续读取并校验哈希，保留每回合初始化检查；关系增量计算仍适用。未宣称 Windows 已获得相同的元数据快路径。
6. 本次不改变持久数据格式、不迁移索引。原有版本不匹配时自动删库重建的行为已改为明确拒绝，遵守仓库单版本数据约束。

本机开发构建、实际 CLI 目录（330 个可索引文件）对照：原单次首次构建 2268 ms、无变化更新 55 ms、单文件更新 779 ms；优化后三次首次构建 2298/2400/2376 ms，无变化更新均 4 ms，单文件更新 254/271/265 ms，就绪检查均 3 ms。单文件变化使用源码副本，未修改真实工程。首次构建基本不变，主要收益是重复访问和增量更新；这些是本机样本，不含模型及权限往返耗时。

真实模型同进程三轮验收通过（隔离交互式 CLI，`qwen3.8-flash-next-iq3_s`）：首次查询被阻拦后建索引，第二轮查询直接复用，第三轮在外部同长度改写并恢复 mtime 后重新阻拦、增量更新并读取新值 `steelblue`。三轮 `codebase_index` 调用次数分别为 1/0/1，第三轮报告 1 个变化文件、20 个未变化文件。该结果验证实际跨回合复用及失效恢复；夹具与数据库均独立于用户数据。
