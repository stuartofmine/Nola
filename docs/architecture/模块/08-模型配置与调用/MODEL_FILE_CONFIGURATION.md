> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 多端点文件配置与 Core 模型目录

简体中文 · [English](MODEL_FILE_CONFIGURATION.en.md)

## 所有权

CLI 对话、init、wiki、Web 和 server 启动时读取生效的模型目录。文件声明连接、模型与宿主默认选择，Core 保存供各端使用的运行状态。已有会话继续使用自己的模型绑定。检查、查看和编辑命令支持在启动前修复文件。

示例见 `apps/cli/config.multi-endpoint.example.json`。每个连接声明 id/name/endpoint/protocol/models；协议为 chat_completions、responses 或 anthropic。模型 id 为服务端原始标识，不按模型名称推测能力。可显式声明 capabilities；未提供能力时使用 Core agent 基线（toolCalls=true，其余能力不推断）。显式false保留；工具能力不替代执行权限。密钥通过 api_key_env 指定环境变量，应用时写入 Host 私有 vault，公开配置回执与 Core 目录不含密钥。匿名端点省略 api_key_env。

需要密钥时，在用户配置中声明服务地址、协议和 `api_key_env`。项目配置可复用相同用户声明，或沿用同一连接已经绑定的有效凭据。单独打开项目不会把任意环境变量绑定到项目指定的服务。切换服务地址或协议时需要为新路由提供凭据。

模型发现解析provider目录元数据：supported_parameters中的tools/reasoning/reasoning_effort/temperature、architecture.input_modalities、context_length、top_provider.max_completion_tokens。已有导入格式的显式字段优先。provider返回完整支持列表时以列表为准，未提供列表不能误判为不支持。启动时应用目录只访问本地宿主；联网元数据由模型发现入口读取。服务不返回的参数不能声称自动获取，具体生成参数也不等于服务能力上限。

## 流程与输入输出

1. 有界、禁止符号链接的绝对文件路径读取，解析 JSON/JSONC 的 llm 对象；拒绝混用单路由字段、未知字段、重复连接/模型、无效默认引用。
2. 全部连接先验证身份、网关回环、账户所有权、环境密钥是否存在，再开始写入。
3. Core 在一个事务中核对连接和账户修订，更新声明的模型、启用状态以及默认选择。任意一步失败都回滚本批次的目录修改。
4. 相同声明、启用状态和凭据保持原有修订号。回执包含连接 ID、默认模型配置 ID 和修订号。
5. 新会话读取已提交的宿主默认模型。若目录被拒绝，CLI 指出出错文件，有可用的已保存默认模型时继续；首次使用且缺少默认模型时，在发送请求前退出。现有会话恢复其原有选择。

文件检查与启动共用 JSONC 解析器和运行配置合并逻辑。预览只读取文件，公开连接 ID、模型 ID、协议、服务来源与凭据环境变量名称。应用时再次使用有界文件读取、Core 连接修订检查及 Host 私有凭据写入。查看配置通过 Application 查询已保存的默认值，可与文件声明对照。没有数据库时查看和预览都不会创建数据库。

## 使用

编辑 `~/.nola/config.json`，保留其他业务配置，将 llm 整体替换成多连接对象。设置声明的密钥环境变量后执行：

```sh
nola config check
nola config
nola
```

会话内使用 `/model` 选择 Core 目录中的模型，列表同时显示连接身份以区分同名模型；选择后端点、协议、凭据与模型整体切换，并写入当前会话，而不是只替换模型名。

使用已应用的目录时，`nola init`、`nola wiki` 和 `nola --no-save` 在执行时读取同一个 Core 默认模型，解析已保存的协议和凭据。这三个入口都不创建会话历史。删除文件里的连接不会删除 Core 记录；删除、停用和凭据解除由模型中心明确操作。单端点即时路由与多端点目录不得混在同一个 llm 对象。

## 默认引用错误定位

`default_model must refer to a declared connection and model` 来自文件预验证，不是联网或 API Key 错误。`llm.default_model.connection` 必须等于同一文件中某个连接的 `id`，`model` 必须等于该连接内某个模型的 `id`；显示名称、其他连接的模型及 Core 中已有但未在此文件声明的模型都不能代替。

例如仓库示例声明 `primary` 下的 `fast-model`，对应引用为：

```json
{"llm":{"connections":[{"id":"primary","name":"Primary service","endpoint":"https://primary.example/v1","protocol":"chat_completions","models":[{"id":"fast-model","name":"Fast"}]}],"default_model":{"connection":"primary","model":"fast-model"}}}
```

对照修正文件中的精确 ID，保存后重新启动 Nola。此错误在开始应用连接前拒绝；不会替用户猜测引用、重写配置或删除已有目录。完整格式以[仓库示例](../../../../apps/cli/config.multi-endpoint.example.json)和[解析器](../../../../apps/cli/src/model_file_config.rs)为准。

## CLI 与 Web 数据库

两端必须绑定同一个 NOLA_DB_PATH 才能共享目录、默认选择及会话。该环境变量是显式Host存储身份，不自动搜索、复制或合并其他数据库。`nola web` 默认与同环境的CLI一致；定制启动脚本若指定不同路径，属于不同Host。应用配置应使用目标Host相同的数据库路径。

## 验收

覆盖多端点同名模型、错误默认引用、重复身份、混合旧路由字段、预验证失败无部分写入、复用Core目录与重复应用、默认切换、CLI启动。配置回归测试使用隔离数据库。`scripts/e2e/test_cli_config_files.py` 使用本地 HTTP 模型端点，验证首次启动、重复启动、拒绝无效目录后继续对话、密钥轮换以及 Web 和 server 启动。

2026-10-19 验证：`cargo test -p nola_cli --lib model_ --offline` 271通过、1忽略；CLI与Application Service构建成功（既有链接器eh_frame警告）。真实PTY在隔离库中完成两连接三模型导入、默认large启动、切换到另一连接的Responses/fast、退出后按session ID恢复。数据库核验选中`config-two|responses|fast`，runs=0。未调用真实模型，也未验证远程服务可达性。

跨协议修复：新建远程模型不默认注入topK或repetitionPenalty；空停止词列表归一为未设置，非空不支持参数仍拒绝。避免首次切换成功而会话持久化后因协议参数污染无法恢复。
