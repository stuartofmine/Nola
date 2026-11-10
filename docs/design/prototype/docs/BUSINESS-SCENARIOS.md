# 全业务场景与状态目录

更新：2026-11-10。主入口：[可操作场景原型](../scenarios.html)；原有完整交互原型：[index.html](../index.html)。

## 范围与使用方法

本清单以当前仓库用户可见业务为范围，共 **68 个场景、220 个状态、6 个平台视角**。每项有独立状态图、原型入口、适用平台与生产源码依据；Web 当前 **14 个路由**全部关联。它覆盖可枚举的业务主线、异常和恢复，不声称穷尽任意数据、语言、系统版本和网络组合。

选择业务与平台，直接操作预览中的按钮；右侧可跳到任意状态。每个状态有独立 URL，浏览器前进／后退可复现。原型目录和状态面板属于评审工具，不是产品导航。字段只保存为当前页面内、当前平台及场景的草稿；取消保留已保存基线，切换场景不覆盖其他草稿，刷新重置。破坏性样例先打开可取消的确认框。没有模型、账户、Host、文件、下载或系统权限请求，不能输入真实凭据。

“适用平台”表示这项设计应在哪里提供，**不是已实现或已测试的能力声明**。不适用时明确显示范围，不生成假系统入口。macOS/iOS/Android 的控件只是浏览器布局示意；Windows 表示共享 Web Host 客户端方向；CLI 为文本布局草图，下方按钮仅用于评审，不是新 CLI 命令。Web 更新安装、手机永久后台调度及跨 Host 接管不作为已支持能力。

## 共同行为

- 桌面检查器原地独立展开，没有二级返回栈；知识、Wiki、历史召回不混成一种对象。
- 手机输入紧凑，保留系统返回与显式按钮；五档模型只显示一组实际推理档位。
- 正文以平台字体为默认，主题决定边框和焦点；回复复制无常驻填充，悬停与键盘焦点均可见。
- 新增内容短暂浮现，历史不重播；尊重减少动态效果，不靠动画延迟模型完成。原型不声称验证原生侧滑或语音识别。
- 授权显示具体操作、位置和范围。缺少安全审阅文本时禁用允许，仍可拒绝；配对、Skill/MCP 启用与工具授权分别处理。
- 客户端离开不取消 Host 任务；恢复读取原 Run。停止等待权威确认；未知写入结果先核对，不能自动重放。
- 上下文占用、压缩效率、缓存命中率分开。未上报用量不得显示成 0%；一次执行结束不等于 Goal 完成。
- Nola 自有版本 1 基线不变；未知格式拒绝，保护原始数据，不自动删除或重建。外部标准协议沿用自身版本。

## 场景覆盖

| ID | 业务 / 场景 | 入口 | 状态数 | 适用设计平台 |
| --- | --- | --- | ---: | --- |
| [welcome](../scenarios.html?scene=welcome&platform=mac) | 开始与账户 / 首次使用与跳过引导 | 首次启动 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [web-login](../scenarios.html?scene=web-login&platform=web) | 开始与账户 / Web / Windows 连接登录 | Host 地址 → 登录 | 3 | Web / Windows |
| [account-browser](../scenarios.html?scene=account-browser&platform=mac) | 开始与账户 / ChatGPT 授权与网页回跳 | 模型 → 添加 ChatGPT 账户 | 4 | macOS / CLI |
| [account-reauth](../scenarios.html?scene=account-reauth&platform=mac) | 开始与账户 / 账户失效、重新授权与退出 | 模型 → 账户详情 | 3 | macOS / CLI |
| [conversation-list](../scenarios.html?scene=conversation-list&platform=mac) | 对话与输入 / 会话列表、搜索与草稿 | 对话 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [compose](../scenarios.html?scene=compose&platform=mac) | 对话与输入 / 普通消息与流式阅读 | 对话 → 输入 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [stream-reader](../scenarios.html?scene=stream-reader&platform=mac) | 对话与输入 / 长回复、代码表格与阅读跟随 | 会话 → 长回复 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [edit-resend](../scenarios.html?scene=edit-resend&platform=mac) | 对话与输入 / 编辑已发送内容与分支 | 用户消息 → 编辑重发 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [copy](../scenarios.html?scene=copy&platform=mac) | 对话与输入 / 复制成功与手动回退 | 回复下方 → 复制 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [image-paste](../scenarios.html?scene=image-paste&platform=mac) | 对话与输入 / 图片、文件粘贴与导入失败 | 输入 → 添加附件 / 粘贴 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [voice](../scenarios.html?scene=voice&platform=ios) | 对话与输入 / 语音输入与麦克风权限 | 输入 → 语音 | 3 | iOS / iPadOS / Android |
| [generation-failure](../scenarios.html?scene=generation-failure&platform=mac) | 对话与输入 / 模型请求失败与恢复 | 会话 → 请求失败 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [conversation-delete](../scenarios.html?scene=conversation-delete&platform=mac) | 对话与输入 / 重命名、导出与删除会话 | 会话 → 管理 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [goal-input](../scenarios.html?scene=goal-input&platform=mac) | 目标与长任务 / Goal 输入、范围与验收引导 | 会话 → 目标 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [goal-budget](../scenarios.html?scene=goal-budget&platform=mac) | 目标与长任务 / Goal 预算用尽与继续 | 目标 → 本次执行 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [goal-human](../scenarios.html?scene=goal-human&platform=mac) | 目标与长任务 / 等待补充输入与人工验收 | 活动 → 需要你回应 | 4 | macOS / iOS / iPadOS / Android / Web / Windows |
| [long-run](../scenarios.html?scene=long-run&platform=mac) | 目标与长任务 / 宿主机执行长任务 | 会话 → 发送长任务 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [remote-reconnect](../scenarios.html?scene=remote-reconnect&platform=mac) | 目标与长任务 / 离开、断线、强制停止与恢复 | 运行中 → 退出客户端 → 返回 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [host-cold](../scenarios.html?scene=host-cold&platform=mac) | 目标与长任务 / 宿主机重启后的上下文恢复 | 重启 Host → 打开原会话 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [cancel](../scenarios.html?scene=cancel&platform=mac) | 目标与长任务 / 停止、物理取消与后续对话 | 会话 → 停止 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [unknown-effect](../scenarios.html?scene=unknown-effect&platform=mac) | 目标与长任务 / 未知执行结果与避免重复副作用 | 执行中断 → 原会话 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [compaction](../scenarios.html?scene=compaction&platform=mac) | 目标与长任务 / 上下文占用、压缩与缓存统计 | 会话信息 → 上下文 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [background-jobs](../scenarios.html?scene=background-jobs&platform=mac) | 目标与长任务 / 后台作业与日志 | 执行过程 → 后台任务 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [model-empty](../scenarios.html?scene=model-empty&platform=mac) | 模型与账户配置 / 没有模型与添加来源 | 设置 → 模型与账户 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [api-config](../scenarios.html?scene=api-config&platform=mac) | 模型与账户配置 / API 连接、验证与保存失败 | 模型 → 添加 API 服务 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [catalog-select](../scenarios.html?scene=catalog-select&platform=mac) | 模型与账户配置 / 目录发现、启用与默认模型 | 模型 → 连接详情 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [config-default](../scenarios.html?scene=config-default&platform=mac) | 模型与账户配置 / 配置导入与默认引用错误 | 模型 → 导入配置 / apply-config | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [reasoning](../scenarios.html?scene=reasoning&platform=mac) | 模型与账户配置 / 实际推理档位、继承与未知能力 | 模型选项 → 推理档位 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [prompt-template](../scenarios.html?scene=prompt-template&platform=mac) | 模型与账户配置 / 预设提示词与作用范围 | 模型配置 → 提示词 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [model-probe](../scenarios.html?scene=model-probe&platform=mac) | 模型与账户配置 / 付费探测与用户能力覆盖 | 模型详情 → 测试能力 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [local-model](../scenarios.html?scene=local-model&platform=mac) | 模型与账户配置 / 本地模型、下载与设备容量 | 模型 → 本地 | 3 | macOS / iOS / iPadOS / Android |
| [host-pair](../scenarios.html?scene=host-pair&platform=mac) | 宿主机与权限 / 粘贴配对、检查与连接 | 模型 / 连接 → 宿主机 | 3 | macOS / iOS / iPadOS / Android |
| [share-host](../scenarios.html?scene=share-host&platform=mac) | 宿主机与权限 / 共享此 Mac 与局域网地址 | Mac → 设置 → 宿主机连接 | 3 | macOS |
| [host-network](../scenarios.html?scene=host-network&platform=mac) | 宿主机与权限 / 离线、地址变更与凭据写入失败 | 宿主机 → 连接失败 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [permission](../scenarios.html?scene=permission&platform=mac) | 宿主机与权限 / 具体操作授权、拒绝与补充指引 | 工具执行前 → 权限申请 | 5 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [permission-missing](../scenarios.html?scene=permission-missing&platform=mac) | 宿主机与权限 / 缺失详情、过期申请与错误身份 | 原会话 → 权限申请 | 4 | macOS / iOS / iPadOS / Android / Web / Windows |
| [system-permission](../scenarios.html?scene=system-permission&platform=mac) | 宿主机与权限 / 系统权限拒绝与设置入口 | 使用受保护资源时 | 3 | macOS / iOS / iPadOS / Android |
| [execution-scope](../scenarios.html?scene=execution-scope&platform=mac) | 宿主机与权限 / 执行位置、设备桥接与边界 | 会话详情 → 执行 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [skill-editor](../scenarios.html?scene=skill-editor&platform=mac) | Skills、MCP 与插件 / Skill 新增、编辑与内置恢复 | 设置 → Skills | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [skill-review](../scenarios.html?scene=skill-review&platform=mac) | Skills、MCP 与插件 / Skill 设置复核与来源保留 | Skills → 需要复核 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [mcp-config](../scenarios.html?scene=mcp-config&platform=mac) | Skills、MCP 与插件 / MCP HTTP / stdio 配置与检查 | 设置 → MCP → 添加 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [mcp-uncertain](../scenarios.html?scene=mcp-uncertain&platform=mac) | Skills、MCP 与插件 / MCP 保存未确认与重新核对 | MCP → 保存 | 4 | macOS / iOS / iPadOS / Android / Web / Windows |
| [mcp-invoke](../scenarios.html?scene=mcp-invoke&platform=mac) | Skills、MCP 与插件 / MCP 调用、外部服务权限与失败 | 会话 → MCP 工具 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [plugin](../scenarios.html?scene=plugin&platform=cli) | Skills、MCP 与插件 / 插件安装、凭据引用与停用 | CLI → plugin | 3 | CLI |
| [data-sources](../scenarios.html?scene=data-sources&platform=mac) | 资料、知识与记忆 / 资料来源、添加与访问策略 | 资料 → 数据源 | 4 | macOS / iOS / iPadOS / Android / Web / Windows |
| [knowledge](../scenarios.html?scene=knowledge&platform=mac) | 资料、知识与记忆 / 会话知识点、编辑与并发冲突 | 会话信息 → 知识 | 4 | macOS / iOS / iPadOS / Android / Web / Windows |
| [wiki](../scenarios.html?scene=wiki&platform=mac) | 资料、知识与记忆 / 独立 Wiki 编辑与版本 | 资料 → Wiki | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [recall](../scenarios.html?scene=recall&platform=mac) | 资料、知识与记忆 / 压缩摘要、历史召回与来源 | 会话信息 → 上下文 / 历史 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [memory](../scenarios.html?scene=memory&platform=mac) | 资料、知识与记忆 / 个人长期记忆、修正与删除 | 资料 → 长期记忆 | 4 | macOS / iOS / iPadOS / Android / Web / Windows |
| [health](../scenarios.html?scene=health&platform=mac) | 资料、知识与记忆 / 健康数据、授权与同步 | 资料 → 健康 | 3 | macOS / iOS / iPadOS / Android |
| [location-calendar](../scenarios.html?scene=location-calendar&platform=mac) | 资料、知识与记忆 / 位置、日历与设备来源 | 资料 → 设备来源 | 3 | macOS / iOS / iPadOS / Android |
| [experts](../scenarios.html?scene=experts&platform=mac) | 专家、Room 与审阅 / 专家配置、身份、颜色与恢复 | 会话 → 参与者 | 4 | macOS / iOS / iPadOS / Android / Web / Windows |
| [room](../scenarios.html?scene=room&platform=mac) | 专家、Room 与审阅 / Room 创建、加入、定向消息与恢复 | 专家 / Room | 4 | macOS / iOS / iPadOS / CLI |
| [subtask](../scenarios.html?scene=subtask&platform=mac) | 专家、Room 与审阅 / 子任务、执行者与取消 | 会话 → 执行 → 子任务 | 4 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [inspector](../scenarios.html?scene=inspector&platform=mac) | 专家、Room 与审阅 / 检查器全部区段与独立展开 | 桌面右栏 / 手机会话详情 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [artifact-review](../scenarios.html?scene=artifact-review&platform=mac) | 专家、Room 与审阅 / 产物预览、版本与审阅 | 产物 / 审阅 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [git-review](../scenarios.html?scene=git-review&platform=mac) | 专家、Room 与审阅 / Git 差异、提交与推送 | Mac / Web → 工作区审阅 | 4 | macOS / Web / Windows |
| [activity-inbox](../scenarios.html?scene=activity-inbox&platform=mac) | 活动、计划与通知 / 活动聚合、筛选与精确来源 | 活动 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [plans](../scenarios.html?scene=plans&platform=mac) | 活动、计划与通知 / 计划创建、编辑、停用与历史 | 活动 → 日历 / 计划 | 4 | macOS / iOS / iPadOS / Android / Web / Windows |
| [event-wait](../scenarios.html?scene=event-wait&platform=mac) | 活动、计划与通知 / 等待事件与等待人的区别 | 活动 → 目标 / 执行详情 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [notifications](../scenarios.html?scene=notifications&platform=mac) | 活动、计划与通知 / 通知权限、中心与详情 | 通用 → 通知 / 活动 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [results-dashboard](../scenarios.html?scene=results-dashboard&platform=web) | 活动、计划与通知 / 结果、概览与今日视图 | 活动 → 结果 / 概览 | 3 | Web / Windows |
| [general-fonts](../scenarios.html?scene=general-fonts&platform=mac) | 通用、隐私与维护 / 通用：字体、主题、语言与通知 | 设置 → 通用 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |
| [updates](../scenarios.html?scene=updates&platform=mac) | 通用、隐私与维护 / 软件更新：检查、失败与安装 | 设置 → 通用 → 软件更新 | 4 | macOS / iOS / iPadOS / Android / Windows |
| [unsupported-data](../scenarios.html?scene=unsupported-data&platform=mac) | 通用、隐私与维护 / 未知协议、ABI 与数据库格式 | 启动 / 连接 / 更新兼容检查 | 3 | macOS / iOS / iPadOS / Android / Web / Windows / CLI |
| [privacy-export](../scenarios.html?scene=privacy-export&platform=mac) | 通用、隐私与维护 / 隐私、导出与清理确认 | 设置 → 隐私与数据 | 4 | macOS / iOS / iPadOS / Android / Web / Windows |
| [diagnostics](../scenarios.html?scene=diagnostics&platform=mac) | 通用、隐私与维护 / 诊断、日志与存储空间 | 设置 → 诊断 | 3 | macOS / Web / Windows / CLI |
| [shortcuts-accessibility](../scenarios.html?scene=shortcuts-accessibility&platform=mac) | 通用、隐私与维护 / 键盘、侧滑、大字与减少动态效果 | 任一页面 | 3 | macOS / iOS / iPadOS / Android / Web / Windows |

## 路由关联

| 生产 Web 路由 | 对应业务场景 |
| --- | --- |
| `/models` | [model-empty](../scenarios.html?scene=model-empty&platform=web), [api-config](../scenarios.html?scene=api-config&platform=web), [catalog-select](../scenarios.html?scene=catalog-select&platform=web), [host-pair](../scenarios.html?scene=host-pair&platform=web) |
| `/model-test` | [model-probe](../scenarios.html?scene=model-probe&platform=web) |
| `/login` | [web-login](../scenarios.html?scene=web-login&platform=web) |
| `/capabilities` | [skill-editor](../scenarios.html?scene=skill-editor&platform=web), [skill-review](../scenarios.html?scene=skill-review&platform=web), [mcp-config](../scenarios.html?scene=mcp-config&platform=web), [mcp-uncertain](../scenarios.html?scene=mcp-uncertain&platform=web), [mcp-invoke](../scenarios.html?scene=mcp-invoke&platform=web) |
| `/activity` | [activity-inbox](../scenarios.html?scene=activity-inbox&platform=web), [event-wait](../scenarios.html?scene=event-wait&platform=web) |
| `/results` | [results-dashboard](../scenarios.html?scene=results-dashboard&platform=web) |
| `/dashboard` | [results-dashboard](../scenarios.html?scene=results-dashboard&platform=web) |
| `/data-sources` | [data-sources](../scenarios.html?scene=data-sources&platform=web), [knowledge](../scenarios.html?scene=knowledge&platform=web), [wiki](../scenarios.html?scene=wiki&platform=web), [memory](../scenarios.html?scene=memory&platform=web) |
| `/settings` | [general-fonts](../scenarios.html?scene=general-fonts&platform=web), [updates](../scenarios.html?scene=updates&platform=web), [privacy-export](../scenarios.html?scene=privacy-export&platform=web), [diagnostics](../scenarios.html?scene=diagnostics&platform=web) |
| `/scheduled-actions` | [plans](../scenarios.html?scene=plans&platform=web) |
| `/workspace-review` | [git-review](../scenarios.html?scene=git-review&platform=web), [artifact-review](../scenarios.html?scene=artifact-review&platform=web) |
| `/today` | [results-dashboard](../scenarios.html?scene=results-dashboard&platform=web) |
| `/conversations` | [conversation-list](../scenarios.html?scene=conversation-list&platform=web), [conversation-delete](../scenarios.html?scene=conversation-delete&platform=web) |
| `/chat` | [compose](../scenarios.html?scene=compose&platform=web), [stream-reader](../scenarios.html?scene=stream-reader&platform=web), [goal-input](../scenarios.html?scene=goal-input&platform=web), [permission](../scenarios.html?scene=permission&platform=web), [remote-reconnect](../scenarios.html?scene=remote-reconnect&platform=web), [compaction](../scenarios.html?scene=compaction&platform=web), [inspector](../scenarios.html?scene=inspector&platform=web) |

## 维护与验收

`BUSINESS-SCENARIOS.json` 为当前目录的场景清单来源；`sync_business_scenarios.py` 校验状态图、源码路径与 Web 路由并生成本文和 CSV。新增业务需同时增加清单和原型状态；新增生产路由但没有映射时检查失败。`--check` 只检查同步，不修改文件。

```sh
python3 docs/design/prototype/sync_business_scenarios.py --check
python3 -m http.server 8071 --bind 127.0.0.1 --directory docs/design/prototype
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_business_scenarios.cjs
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_business_layout.cjs
```

浏览器验收范围与实际计数见 [VALIDATION.md](VALIDATION.md)。原型通过不代表生产链路通过；真实 Host 长任务的独立结果见 [宿主机流程](../../shared/HOST_CONNECTION_WORKFLOW.md)。外部旧 Site 未随本地原型自动发布。
