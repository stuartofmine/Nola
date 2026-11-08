> **单版本开发约束：仅保留当前协议；不迁移、转换或自动重建用户数据。外部模型服务协议适配不属于旧版本兼容。**

# 多端点文件配置与 Core 模型目录

## 所有权

文件是显式编辑入口，Core ModelConnection、CatalogModel、ModelProfile 与 chat.default_model 是运行权威。不把 connections 数组当成故障回退列表，不在每次启动时覆盖模型中心编辑。显式执行 `nola model apply-config [absolute-config-path]` 才写入；省略路径时只读取 `~/.nola/config.json` 或 `.jsonc`；已有会话继续读取自己的冻结选择与覆盖参数。

示例见 `cli/config.multi-endpoint.example.json`。每个连接声明 id/name/endpoint/protocol/models；协议为 chat_completions、responses 或 anthropic。模型 id 为服务端原始标识，不按模型名称推测能力。可显式声明 capabilities；未提供能力时使用 Core agent 基线（toolCalls=true，其余能力不推断）。显式false保留；工具能力不替代执行权限。密钥通过 api_key_env 指定环境变量，应用时写入 Host 私有 vault，公开配置回执与 Core 目录不含密钥。匿名端点省略 api_key_env。

首次导入需要提供声明的密钥环境变量。重新应用时，若环境变量不可用，可复用同来源、同 URL/协议/账户且状态为 Ready 的已有凭据引用，不重新写 vault。省略 api_key_env 不表示删除已有凭据；删除/撤销通过账户管理显式操作。更换 URL 或协议不能自动沿用旧凭据，必须显式提供。引用可复用不等于远端认证一定成功，运行时仍验证私有存储与实际服务响应。

模型发现解析provider目录元数据：supported_parameters中的tools/reasoning/reasoning_effort/temperature、architecture.input_modalities、context_length、top_provider.max_completion_tokens。已有导入格式的显式字段优先。provider返回完整支持列表时以列表为准，未提供列表不能误判为不支持。apply-config仍是离线显式配置操作，不隐式访问每个端点；联网元数据读取走模型发现入口。服务不返回的参数不能声称自动获取，具体生成参数也不等于服务能力上限。

## 流程与输入输出

1. 有界、禁止符号链接的绝对文件路径读取，解析 JSON/JSONC 的 llm 对象；拒绝混用单路由字段、未知字段、重复连接/模型、无效默认引用。
2. 全部连接先验证身份、网关回环、账户所有权、环境密钥是否存在，再开始写入。
3. 每个连接复用现有带 revision 的 SaveConnection、RefreshConnectionCatalog、ConfigureConnectionModels 与账户凭据保存流程。连接跨批不是原子事务；后项失败明确报告已应用的连接，不伪报全部成功或自动重放。
4. 全部成功后设置 Core chat.default_model；回执包含连接ID、默认profile ID和revision，不包含secret。
5. CLI配置中有 connections 时，新会话读取 Core 当前默认选择与revision；未应用、默认模型禁用/丢失时明确报错。模型中心改默认值无需改文件；再次apply才覆盖该默认值。显式 --profile 优先；禁止混入 --endpoint/--model/--api-key 造成不明确的身份。现有会话不受默认变更影响。

## 使用

编辑 `~/.nola/config.json`，保留其他业务配置，将 llm 整体替换成多连接对象。设置声明的密钥环境变量后执行：

```sh
nola model apply-config /Users/gaojc/.nola/config.json
nola model connections
nola
```

会话内使用 `/model` 选择 Core 目录中的模型，列表同时显示连接身份以区分同名模型；选择后端点、协议、凭据与模型整体切换，并写入当前会话，而不是只替换模型名。

不把该目录用于 init/wiki/no-save 的隐式路由；这些入口仍要求显式单路由配置。删除文件里的连接不会删除 Core 记录；删除、停用和凭据解除由模型中心明确操作。单端点即时路由与多端点目录不得混在同一个 llm 对象。

## 默认引用错误定位

`default_model must refer to a declared connection and model` 来自文件预验证，不是联网或 API Key 错误。`llm.default_model.connection` 必须等于同一文件中某个连接的 `id`，`model` 必须等于该连接内某个模型的 `id`；显示名称、其他连接的模型及 Core 中已有但未在此文件声明的模型都不能代替。

例如仓库示例声明 `primary` 下的 `fast-model`，对应引用为：

```json
{"llm":{"connections":[{"id":"primary","name":"Primary service","endpoint":"https://primary.example/v1","protocol":"chat_completions","models":[{"id":"fast-model","name":"Fast"}]}],"default_model":{"connection":"primary","model":"fast-model"}}}
```

对照修正文件中的精确 ID，再执行 `nola model apply-config /absolute/config.json`。此错误在开始应用连接前拒绝；不会替用户猜测引用、重写配置或删除已有目录。完整格式以[仓库示例](../../../../cli/config.multi-endpoint.example.json)和[解析器](../../../../cli/src/model_file_config.rs)为准。

## CLI 与 Web 数据库

两端必须绑定同一个 NOLA_DB_PATH 才能共享目录、默认选择及会话。该环境变量是显式Host存储身份，不自动搜索、复制或合并其他数据库。`nola web` 默认与同环境的CLI一致；定制启动脚本若指定不同路径，属于不同Host。应用配置应使用目标Host相同的数据库路径。

## 验收

覆盖多端点同名模型、错误默认引用、重复身份、混合旧路由字段、预验证失败无部分写入、复用Core目录与重复应用、默认切换、CLI启动。所有测试使用隔离库与无网络fixture，不调用用户本地模型。

2026-10-19 验证：`cargo test -p nola_cli --lib model_ --offline` 271通过、1忽略；CLI与Application Service构建成功（既有链接器eh_frame警告）。真实PTY在隔离库中完成两连接三模型导入、默认large启动、切换到另一连接的Responses/fast、退出后按session ID恢复。数据库核验选中`config-two|responses|fast`，runs=0。未调用真实模型，也未验证远程服务可达性。

跨协议修复：新建远程模型不默认注入topK或repetitionPenalty；空停止词列表归一为未设置，非空不支持参数仍拒绝。避免首次切换成功而会话持久化后因协议参数污染无法恢复。
