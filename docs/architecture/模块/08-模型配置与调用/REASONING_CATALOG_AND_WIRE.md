# 推理档位：目录、选择与请求

推理支持、可选档位、推理开关、token 预算和公开摘要是不同能力。档位属于具体连接、模型及协议，不是全局枚举；`automatic` 表示省略控制、采用提供商默认，显式 `none` 只有服务端声明支持时才能发送。不能凭模型名猜档位，也不能在请求失败后静默改成 `none`、其他档位或另一种协议。

## 参考实现核对

2026-11-14 核对了以下固定版本，避免本地参考副本过旧：

- OpenCode `f6e0c2d0833ffe772035f221fed923e1cd5fb408`：[models-dev.ts](https://github.com/anomalyco/opencode/blob/f6e0c2d0833ffe772035f221fed923e1cd5fb408/packages/core/src/models-dev.ts) 读取 models.dev 的结构化 `capabilities.reasoning`；[variant.ts](https://github.com/anomalyco/opencode/blob/f6e0c2d0833ffe772035f221fed923e1cd5fb408/packages/core/src/variant.ts) 将 effort、toggle、budget 转成协议对应的 variants。提供商默认 `default` 不作为显式 variant，开关关闭不一律编码成 `reasoning_effort=none`。其供应商特定映射不能泛化为所有兼容端点的能力。
- Magpie `6112dd37c14fb6c66d021c8a761f6fa7c9419485`：[接入契约](https://github.com/yetone/magpie/blob/6112dd37c14fb6c66d021c8a761f6fa7c9419485/docs/integrating.md) 公开 `/v1/models` 的 `supported_reasoning_levels` 对象数组及 `reasoning_levels` 字符串数组；[live.go](https://github.com/yetone/magpie/blob/6112dd37c14fb6c66d021c8a761f6fa7c9419485/internal/catalog/live.go) 读取实时目录；[ir.go](https://github.com/yetone/magpie/blob/6112dd37c14fb6c66d021c8a761f6fa7c9419485/internal/gateway/ir.go) 按目标协议转换，并可能拟合到目标模型支持的邻近档位。Nola 是客户端，不照搬网关的静默拟合策略；用户的显式选择必须原值执行或明确拒绝。

## Nola 的端到端约束

1. 目录读取是有界、可取消的 GET，不启动付费推理。读取服务端实际返回的两种档位数组、去重保序；优先使用显式 `supported_reasoning_levels`，空数组不会从另一个字段补造档位。缺失元数据保持未知。
2. Apple 共享发现器及 Rust Host 必须把观察到的档位继续传给 Core，不能只保留 ID 后重建空的 `reasoning_levels`。iOS 首次配置从发现列表选模型时，也必须带入该次同端点、同凭据、同协议的目录结果。手输模型 ID 不触发自动发现。
3. Core 保存原始目录观察，生成 profile 时排除 `default`、`automatic` 和 harness 协作模式 `ultra`；这些不是单模型 wire effort。未知的其他控制值明确拒绝，不替换成较低档位。
4. Chat Completions 和 Responses 接受声明的实际 effort 子集，不要求凑齐 low/medium/high。发布基线已有的 Chat 三档默认配置继续可读；新默认值允许声明的原生 effort。具体模型的失效选择明确报错，不能退回全局默认模型或隐藏改档。
5. 每次请求冻结 profile/revision 和最终选择。Chat Completions 发 `reasoning_effort`；Responses 发 `reasoning.effort`；automatic 不发送二者。选择 `none` 时保留显式字段，不能用 nil 代替。切换模型必须重新按新 profile 校验，不能沿用另一模型的可用档位列表。

Anthropic 的已声明原生 effort 使用 `output_config.effort`，不折算 token 预算，也不从档位列表推断支持 adaptive thinking。automatic 省略控制；显式 none 仅在已声明时编码为 `thinking.type=disabled`。已声明的 token budget 继续使用 `thinking.type=enabled`、`budget_tokens`。独立 adaptive thinking 模式及供应商私有 body overlay 尚未贯通。移动端本地模型适配器仍仅支持 Chat Completions / Responses，Anthropic 通过支持该协议的 Host 执行。

普通 `/v1/models` 如果只有模型 ID 或 `reasoning=true`，不足以证明有哪些可调档位。显式能力声明与目录缺失必须区分；外部目录缓存或 models.dev 只能在明确绑定提供商与协议的适配器内使用，不能用模型名匹配填补任意兼容端点。工具与 reasoning 的兼容性是另一条契约，不能通过自动降档掩盖；改用 Responses 必须是该服务支持且用户明确配置的路由。

## 回归入口

- `nola_domain` reasoning 校验与 `nola_application` model generation：稀疏档位、原生默认值、基线三档默认值、automatic 与 none 分离。
- `model_connections_e2e`：目录观察 → 启用 profile → 冻结选择，原值执行与不支持值拒绝。
- CLI `model_connection_host_tests`：真实 loopback GET 保留元数据，发现本身不选模型或创建 profile。
- Apple `MacSharedProtocolTests` / `IOSSharedProtocolTests`：对象数组、字符串数组、显式空列表、元数据缺失及 Core 序列化。

模型流式 HTTP 客户端对显式回环地址（localhost、IPv4/IPv6 loopback）直连，避免本机系统代理截走本地模型请求；远程主机继续遵循代理设置。代理豁免依据解析后的目标主机，不依据提示词或模型名。
