> **单版本开发约束：仅保留当前架构、接口和存储格式，不保留旧版兼容层，不迁移、删除或重建用户数据。**

# Git 检查与执行权限

## 已实现边界

Shell 的权限不能仅依赖模型声明的 `operation_kind=inspect`，也不能把一次批准提升成整个 Git 可执行文件的权限。

`GitReadPlan` 是 Host 内部的受限执行计划：输入为 Shell 文本、工作目录；输出为规范化仓库路径及固定的执行命令，不能解析的输入返回无计划。权限解析器核对实际仓库路径、敏感性及是否存在可执行 filter，最后产生 `OrdinaryRead` 事实；仓库位于 CLI 工作区外不单独触发目录授权。执行器复用同一计划生成器，而不是批准一套参数、执行另一套参数。

自动只读范围是 `status`、有界 `log`、元数据 `diff`，以及只消费标准输入的受限 `cut/sort/uniq/grep/head/tail/wc` 管道。禁止别名、用户 `-c` 配置、任意程序路径、输出文件选项、文件型过滤参数、Shell 替换和控制流。参数经逐项引用，固定系统程序和环境；关闭分页器、fsmonitor、外部 diff、textconv、签名展示、可选索引写入和网络协议。

linked worktree、外部对象库、clean/process filter 配置、后台执行或其他不支持的形式继续按不透明进程处理。无效配置不得成为只读证据。该计划不是 OS 沙盒，也不承诺防御同权限外部进程在检查和执行间恶意修改仓库。

## 用户授权与模型交互

- CLI 工作区用于默认路径和项目上下文；普通外部仓库无需 `workspace_access`。原生 OS 目录授权不自动授权仓库修改。
- 不透明进程仍只接受单次批准，不把原始命令摘要作为可复用权限。
- `git add/commit`、hooks、签名程序、破坏性修改和 `push` 不继承只读权限；本次没有实现这些操作的仓库级可复用授权。
- `workspace_status` 一次返回清单，模型应复用清单进行提交分组，只有状态变化或证据过期时刷新。复杂检索优先使用已有 `git_history/git_diff` 或持久输出检索，不通过不断改写 Shell 管道重复查询。
- Shell 脚本仍可经明确授权执行；不按用户自然语言强制路由，不通过关键词推断授权。

## 验证

隔离临时仓库测试固定执行程序、普通检查和受限管道、恶意选项、普通外部仓库、敏感资源、fsmonitor/外部 diff、filter 回退，以及权限事实与执行命令的一致性。不在用户仓库执行测试提交，不更改用户权限或 Git 配置。

2026-10-20 的目录与敏感读取规则及验证范围见 [Permission Boundary Contract](PERMISSION_BOUNDARY_CONTRACT.md#filesystem-scope-and-model-requested-sensitive-reads)。包含 `awk`/`sed` 或未受支持控制流的命令仍可能按未知进程效果确认；移除工作区目录限制不等于把所有 Git Shell 文本自动判为只读。
