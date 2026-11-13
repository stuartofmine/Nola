> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Rust 共享编译缓存

更新：2026-10-08。使用 mr-boxington（`mbx`），不改变 Cargo 的依赖图与目标平台。
本轮同时调整开发 profile 和共享产物目录，具体见下方存储策略。
构建仍由 Cargo 决定，mbx 在编译边界缓存
符合条件的 action；跨 checkout 的路径归一化允许共享结果。
依据：[官方工作原理](https://mr-boxington.jdx.dev/how-it-works)。

## 接入合同

主要产品构建脚本共用 `scripts/lib/rust_build_cache.sh`，统一选择执行器。
CLI、Mac 内嵌 runtime、Core Apple/Android 构建接入同一辅助模块。

| 配置 | 行为 |
| --- | --- |
| `NOLA_RUST_BUILD_CACHE=auto`（默认） | 可用时使用 mbx；缺失或调用者设置了 compiler wrapper 时报告原因并使用 Cargo |
| `NOLA_RUST_BUILD_CACHE=off` | 不启用本仓库的 mbx 包装；保留调用者自己的 Cargo/wrapper 配置 |
| `NOLA_RUST_BUILD_CACHE=mbx` 或 `required` | mbx 缺失或与显式 compiler wrapper 冲突时直接失败 |
| `NOLA_MBX_BIN=/absolute/path/mbx` | 指定一个可执行文件，不接受 shell 表达式 |
| `CARGO=/absolute/path/cargo` | mbx 和直接调用使用同一个已解析 Cargo |

显式设置的 `RUSTC_WRAPPER` / `RUSTC_WORKSPACE_WRAPPER` 包括空值，均应尊重。
编译命令使用 `mbx build/test/...`，不是 `mbx cargo ...`；metadata、clean、
帮助等非编译操作仍交给原 Cargo。`mbx clean` 的管理语义不同，不用于替换
用户请求的 Cargo clean。[官方 CLI](https://mr-boxington.jdx.dev/cli)。

CLI 与 Mac 内嵌 runtime 默认共用 workspace `target`，保留显式覆盖选项；
不再默认维护独立的 `target/mac-runtime`。平台构建按 target triple 隔离。
构建脚本不执行 `mbx setup`、全局 Cargo shim、远端缓存配置、target 迁移或清理。
本机经用户授权单独执行 `mbx adopt`，workspace `target` 现在指向受管理缓存。
不修改用户的 shell、mise、编辑器或全局 Cargo 配置。
[官方安装](https://mr-boxington.jdx.dev/installation)、
[setup 行为](https://mr-boxington.jdx.dev/setup)。

默认 dev/test/release 均使用 `debug=0`、`strip="debuginfo"`，关闭 split debuginfo
和 Cargo incremental；bench 继承 release。保留普通链接符号及 dev/test 的断言和溢出检查，
但默认 Rust 回溯不再提供源码行号，LLDB 不提供局部变量调试。
Cargo 指纹仍复用未变化的依赖与产物，不会每次全量重编。
配置变更在下次构建生效，不自动清理现有产物。

完整调试使用 `cargo build --profile dev-debug`。共享 launcher 也默认关闭 mbx
learned/eager incremental，调用者显式的 `MBX_*` 覆盖仍被保留。
仅关闭 Cargo incremental 不会关闭 mbx 自动学习的私有增量状态。
若调用者特意设置`MBX_INCREMENTAL=1`，Cargo的私有增量产物会旁路共享action缓存。
[官方增量编译合同](https://mr-boxington.jdx.dev/incremental)。

共享机器预算仅通过 `sh scripts/configure_rust_cache.sh --apply` 显式设置：
总计 40 GiB，action 12 GiB、managed target 24 GiB、增量 4 GiB；非活跃保留
7 天，回收间隔 15 分钟，磁盘保留 12 GiB。先用 `mbx gc --dry-run` 查看清理计划。
这是软回收预算而非硬配额，活跃构建和临时链接文件仍可能超出；不覆盖未纳管目录。
特别注意：mbx 保留最近使用的整个 target，**不会清理该目录内部的旧 workspace 变体**。
10-09 实测唯一 target 已达 45.4 GiB（debug/deps 约 41 GiB，incremental 为零），
`mbx gc --dry-run` 仍保留它，只计划移除约 738 MiB 的 action 缓存。
因此降低 mbx 预算不能解决这一问题，也不应清空有用的第三方依赖来掩盖它。

本仓库新增显式的局部维护入口（使用 Cargo 自己的包选择与构建锁）：

```sh
sh scripts/clean_rust_workspace.sh          # 默认仅预览
sh scripts/clean_rust_workspace.sh --apply  # 清理 Nola native Debug workspace 产物
```

保留第三方依赖、Release、工具链、mbx actions 和用户数据；当前 Debug 可执行文件也会被清理，
下次运行前需重建。固定作用于本 checkout 的 target，不随外部 `CARGO_TARGET_DIR` 跑到其他项目。
构建入口对显式 target 超过 24 GiB 发出提醒，不在构建过程中自动删除产物。
10-09 本次 Cargo 报告移除 40.8 GiB，target 从约 45 GiB 降至 4.7 GiB（重建前）；
APFS 实测可用空间增加约 39.4 GiB，不能把逻辑清理量与物理释放量混为一谈。
完成 Rust 回归、签名 Debug 与隔离 Test 构建后 target 约 7.7 GiB；构建脚本 41 项测试通过，
包括默认预览、checkout 范围固定、显式应用、超量仅提醒及签名产物的精确 staging 权限。
完整规则见[软件设计文档](../总体设计/SOFTWARE_DESIGN.md#rust-构建与缓存存储策略2026-10-08)。

Core 只生成 Rust `rlib`，原生静态/动态库由 `nola_mobile_bindings` 单独输出。
此前 Core 同时声明三种 crate-type，产生没有指纹后缀的 `deps/libnola_core.rlib`；
Mac 与完整测试的不同特性构建会共用这个文件。r14d 文档测试发生依赖不匹配后，
本批移除这组重复原生产物，保留共享 target，不以新增独立缓存目录规避问题。
修正后的并发构建与文档测试尚待整批验证。

Mac 构建取消必须包含 wrapper 的编译子孙：为本次构建建立独立进程组，
在启动 Cargo 前确认组长身份，退出时只向仍可核验的本组发信号。
禁止按进程名称清理，也不能仅杀 mbx 的一个直接子进程。
Xcode的Core和Runtime两阶段显式将缓存辅助脚本列为输入，生成的dependency file
也包含它；仅修改辅助模块时不能复用旧的已完成phase。
mbx编译子进程遇到超过60字节的`TMPDIR`时使用短`/tmp`，避免其安全随机目录加
Unix socket文件名超出macOS上限；不改调用方环境，直接Cargo和非编译操作不变。
Runtime及Embed使用声明的精确目标文件复制，不使用会创建随机同级临时文件的
BSD `install`；Xcode脚本沙盒保持开启。

## 使用

```sh
# 本机安装后，既有构建入口自动选择 mbx
sh apps/cli/scripts/build.sh --debug

# 明确要求缓存，否则失败
NOLA_RUST_BUILD_CACHE=required sh apps/cli/scripts/build.sh --debug

# 临时关闭仓库包装
NOLA_RUST_BUILD_CACHE=off sh apps/cli/scripts/build.sh --debug

# 本机 workspace target 已显式纳管；新 checkout 先决定是否 adopt
mbx test -p nola_cli --bin nola
mbx stats --json
```

安装是单独步骤，不放进构建脚本。Xcode 没有继承交互式 PATH 时，辅助模块
检查已有的 `~/.local/bin/mbx`、`~/.cargo/bin/mbx`，也可显式设置 `NOLA_MBX_BIN`。

## 实际证据与限制

以下 10-03/10-04 记录描述当时的布局与验收，不代表当前未纳管状态。
10-08 已升级 Rust/Cargo 1.99.0、rustup 1.29.1；保留十二个 target，移除重复
安装的同版本工具链。按 Nola workspace 精确清理 learned 状态，工具报告移除
58.1 GiB 逻辑数据；这些是可重新生成的编译缓存，没有删除会话或用户文件。
新策略的 37 项脚本测试通过，应用级验收另记于
[当前验收记录](../模块/04-存储与索引/COMMITTED_ASSISTANT_JOURNAL_APPLICATION_ADAPTER.md)。

2026-10-03 本机从官方发布安装 Apple Silicon `mbx 1.21.1`，安装前校验 archive
SHA256 与 GitHub release asset digest 一致；未执行全局 setup。
`mbx doctor --json`：0 failure，APFS clone 检查通过；唯一 warning 是普通 Cargo
尚无全局 shim，属于上述局部接入的预期选择。

两个新建、内容相同、路径不同的独立 crate 使用各自全新显式 target：首次构建
存入约 28.3 KiB，第二次实际报告 1 cache hit / 0 miss；Cargo elapsed 为
0.40s / 0.04s。统计显示 managed target 为 0，pruned byte 为 0，远端上传/下载
均为 0。这证明本机跨路径复用，不是 Nola 完整构建性能基准。

本批真实共享层test编译也报告106次缓存命中。mbx输出的避免编译耗时是工具
估算，不是整项目冷/热构建墙钟对照；测试通过数量与后续真实产物构建另行记录。

APFS clone 共享底层块，`du`、逻辑对象大小、实际释放空间不能混为一谈。
不支持clone的平台可能使用只读hardlink；此类target切回直接Cargo时可能报输出
不可写。按[官方恢复策略](https://mr-boxington.jdx.dev/how-it-works#output-restoration)
处理具体输出，仓库不自动清空target或改变全局恢复策略。
现有约 20 GiB 的 debug 目录和 Mac runtime 目录没有删除；不能声称这些旧文件
已经节省出来。mbx 本地 cache 的全局预算/GC 仍由其配置管理，仓库不擅自覆盖。
项目真实构建命中、完整测试、取消隔离的验收结果另记于
整体方案第 8.9 节（本地工作记录，不随仓库分发）。

本机隔离脚本测试27项通过，包含mbx→Cargo→grandchild三个独立活进程的取消与
邻居存活、错误PGID拒绝启动、wrapper/argv/环境/退出码及统一workspace manifest。
fixture不调用真实Cargo，不能代替上述真实项目构建验收。

10-04新增上述Xcode边界验证后，脚本回归32项通过，包含真实macOS沙盒精确输出、
长临时路径隔离、禁用ps的沙盒内进程组检测和exec进程身份。Mac两个runtime实际产物曾重建exit0（45.51s，
484次命中）；不把工具估算节省时间当成墙钟对照。
一次手工`mbx test`漏设`CARGO_TARGET_DIR`，工具将原target整体移入受管理目录并
建立symlink。该轮退出后已原样移回原目录，登记和链接留在隔离临时目录作可恢复
备份；未删除任何编译产物。恢复后的stats确认managed targets及pruned bytes均为0。
这是实际发生并已撤回的布局改变，不是“从未迁移”的证据；后续手工验证同样必须
显式指定target，不能只依靠构建脚本的保护。


## 2026-11：构建范围和产物存储

- 过滤单元测试使用 `bash scripts/test_rust.sh <package> <filter>`，默认限制
  `--lib`。集成测试明确传 `--test <target>`。单独使用测试名称过滤器不会阻止
  Cargo 链接其他集成测试；避免无意生成大量测试可执行文件。
- Apple / Android 原生桥只请求 `cargo rustc --lib --crate-type staticlib`，
  不同时生成未使用的动态库。Mac 本机架构复用 workspace host target；跨 SDK
  缓存仍独立，不能把不同 SDK、features 或编译参数强行视为同一份产物。
- Xcode 脚本调用 Cargo 校验传递依赖，不用归档修改时间绕过 JSON、配置和工具链
  变更。依赖路径批量转义，避免为每个源文件启动一个 sed 进程。复制前比较内容，
  避免未变化的静态库触发 Swift/JNI 重新链接。
- iOS 开发继续复用 release profile，保留运行时需要的优化；不使用曾导致
  初始化故障的 opt-level=0，也不新增一套重复的开发 profile / SDK 依赖缓存。
- `python3 scripts/maintain_build_storage.py` 默认只预览。确认范围后添加
  `--apply`，只移除当前 checkout Debug 中具有匹配 Cargo fingerprint 的过期集成
  测试可执行文件；每个包保留最新 8 个。依赖库、应用、Release、日志及会话不在范围。
  清理会取得 Cargo profile 锁；构建进行中明确拒绝。已删的测试程序下次由 Cargo 重建。

清理器测试包含真实的最小 Cargo 项目：验证锁竞争、保留依赖、删除测试程序后可
重新构建。空间收益以文件系统实际可用空间为准，不用 APFS 逻辑大小冒充物理释放量。
