# Mac ChatGPT 首请求失败修复（2026-11-09）

## 现场与根因

最新正式 Mac 会话输入“你好”，模型为 `gpt-6.1-terra`，通道为 `siwc_responses`。首次 Model Attempt 从 started 到 failed 约 6 ms，没有任何工具执行，公开错误只有 `The model request failed`。

从只读的持久化请求提取消息与 21 个声明进行离线重放，修前明确复现 `SIWC supports only admitted function tools`。Core 给内置工具 envelope 添加布尔型 `x-nola-read-only`，而 `FrozenSiwcTools::from_chat_tools` 只允许 type/function 两个键；它在构造请求 body 时提前拒绝整个目录。原错误投影未覆盖该 Host 预检错误，因此丢失了可操作原因。

已有简化 function schema fixture 不含真实目录的这一标记。普通 Responses/API-key 路径又会重新投影 function，故该缺口主要在 SIWC 的严格 namespace 适配处显现。

## 修改

- 仅允许已知、布尔类型的 Core 内部标记；保留原 canonical 目录，生成 SIWC namespace 时不发送该字段。名称、description、parameters 和 strict 语义不变。
- 未知顶层字段、错误标记类型、重复名称、hosted/custom 工具、未知 namespace/未准入调用继续拒绝。provider wire 不获得接受 Host 内部标记的权限。
- 明确的工具 schema 预检错误保留稳定代码 `invalid_tool_schema`，公开文案给出工具配置/应用更新指引，不复制原始错误或任何账户凭据。
- 不按“你好”等词禁用工具或改路由，不回退 API key/其他账户。协议、ABI、数据库格式编号保持不变，维护重新发布的 1.0.0 基线。

## 验证

1. 原持久化请求离线重放：修前失败、修后完整通过 SIWC body 构造和验证。临时诊断代码已移除，不把原请求正文或账户材料放入仓库。
2. 保留生产目录回归：直接使用 `tool_schemas_for_host` 和初始 Core 投影，不再只测手写简化 schema；“你好”、Hello、Search current model capabilities 原文均保留，同一目录不由文本选择或授权工具。
3. 协议层覆盖 readonly=true/false、原目录不变、namespace 冷重建、不接受错误类型/未知扩展、provider 不接受内部标记、未准入名称不执行。
4. 使用当前所选 ChatGPT 账户与同一模型，重放保存的消息和 21 个工具声明，真实 SIWC 流成功完成，获得 47 字节文本、0 个工具调用。未创建新的用户会话、执行工具、续签/更换凭据或改写原失败记录。初次人工诊断中持有的测试句柄曾占用账户锁，已在重放前明确释放；这不是原请求失败的判据。
5. 39 项 Host Protocol 单元测试、27 项 SIWC 回归及 1 项错误代码定向回归通过；Mac 正常签名 Debug 构建通过，`codesign --verify --deep --strict` 校验通过。

这确认了首条实际请求的适配与流结束路径，不把它扩大为全部工具权限、所有账户/模型、Android/iOS 或多轮产品界面的验证。原失败记录按真实历史保留，应用更新后重试才产生新的结果。

## Mac 构建

`xcodebuild -project Nola.xcodeproj -scheme NolaMac -configuration Debug -destination 'platform=macOS,arch=arm64' -destination-timeout 10 build` 完成。未关闭代码签名、未将 Test 产物覆盖到 Debug。签名校验通过的内置 `nola-agent-host` SHA-256 为 `5d451cf3e302ad41097b7a985ba348faa34ae656a0dcbd3d813cc60ba3d277c3`。

正式数据库只读复核仍为原来的 1 条会话、1 次运行，原失败记录保留。编译产物更新后应重新打开 Debug 开发版，再重试原消息；历史失败不会被伪改成成功。
