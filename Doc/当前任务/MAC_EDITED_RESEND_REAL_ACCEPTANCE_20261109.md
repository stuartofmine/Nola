# Mac 编辑已发送消息：真实模型复现与验收（2026-11-09）

## 为什么补这一轮

前两轮主要是边栏状态、布局和协议专项，没有覆盖普通消息从真实发送、编辑弹窗到重发的完整产品链路。这不能作为编辑功能可用的证据。本轮使用维护者要求的真实模型调用与现有选中连接，隔离数据、账户配置和测试目录；没有转换或重放用户旧开发数据库，也没有创建/修改生产凭据。

## 真实失败与修复

首次测试通过普通输入框发送请求，真实上游 Responses 返回 HTTP 200，随后实际点击编辑按钮、在 AppKit 编辑器修改文本并点击保存。弹窗失败为 `invalid chat_output_token_field`。证据 `/tmp/nola-mac-real-memory.xQxXde/result.xcresult`、`/tmp/nola-edit-first-summary.json`，首轮失败保留。

冻结路由的 `output_token_field` 是物理协议字段；Mac 将 Responses 的 `max_output_tokens` 原样写入只接受 `auto/max_tokens/max_completion_tokens` 的 Chat preference，Host 在读取请求时拒绝。修复是明确适配：Responses `max_output_tokens` 和 SIWC `omitted` 对应 Chat preference `auto`；Chat 两个显式字段原样保留。完整 frozen output admission 和原模型路由不变，Host 私有工厂继续恢复并核对精确原始绑定。

另外修复两处入口缺口：

- ChatGPT 账户原先被 Mac 适配器直接判为 unsupported。现在从当前唯一可用模型和 Core 账户校验原 account ID/revision/credential reference/ready 状态，保留 SIWC profile selection，启动前再复核；不读取 API key、不增加 fallback。Host 仍核验 client、subject 和私有会话。
- 缓存键只有摘要时原先也被 Swift 拒绝。现在由已有私有 Host 工厂读取并校验原始缓存材料；Swift 不反推摘要、不丢弃缓存字段、不降低 Host 校验。

## 真正执行的产品交互

`testRealOrdinaryMessageEditResendFollowupAndColdRestore` 实际执行：普通发送、打开编辑器并验证原文、取消编辑、确认没有新增会话且原草稿保留、再次编辑并保存、分支内追问、返回原会话确认原文/草稿不变、第二次编辑创建独立分支，以及用新 Repository 冷读历史。通过 Core 原始调用、实际 UI 和持久化结果交叉检查。

`testRealStreamingSurvivesConversationSwitchAndKeepsLatestVisible` 请求 60 条中文长回复，在真实生成途中新建会话并写入未发送草稿，切回原会话，检查增量呈现、停止/发送状态、完整终稿和恢复。

批量测试的 transcript 改为保留各用例最新证据再合并，避免最后一例覆盖前例。终局验收依然比较 Application 生产验证结果和数据库不可变 seal，并拒绝缺少或未核算的 Run，没有调低成功门槛。

## 已取得的真实证据

- `/tmp/nola-real-edited-resend-fixed.log`：修复后的首次真实编辑链路通过，4 个 answered seals。
- `/tmp/nola-real-edit-and-stream-verified.log`：2 个真实 UI 用例通过、0 跳过；5 个生产验证且落盘的 answered seals，0 个失败 Run。
- 对应目录 `/tmp/nola-mac-real-memory.pOpKMM/`。五个流式响应均为 HTTP 200；短交互分别产生 4/4/4/5 个文本增量，长回复为 1970 个增量、8152 UTF-8 字节；UI 时间线 354 个采样，353 个在生成中，最终停留在完成态。
- 一轮新增取消验证的测试曾因 Swift AX selector 返回类型推断歧义而编译失败；已显式声明 Any?，没有绕过实际编辑器读取或删除原文断言。
- `/tmp/nola-mac-edit-regression.log`：86 项 Mac 专项通过，包括原有检查器、Journal、编辑快照、ChatGPT profile 和 credential gate；账户断开后必须拒绝重发，缓存快照不再被 Swift 拒绝。测试仍使用 Test 配置。

真实联网验收使用现有 `gpt-6.1-terra` Responses API 连接。ChatGPT 账户分支完成真实 Core 账户/模型夹具回归，没有为验收重新登录真实 ChatGPT 账户；不把这项夹具验证写成已完成真人授权联网测试。未执行跨平台全仓库验收。

## 可重复运行

`scripts/test_real_mac_edited_resend.sh` 使用同一真实 UI 双用例及严格终局检查，要求现有 Nola 配置声明 connections/default_model 和相应 credential environment。默认隔离 Store，不读取用户会话作为测试输入。测试日志和证据均不进入源码包。

最后一轮额外收紧追问：提示中不提供计算答案，须使用编辑分支的上一轮结果继续；同时等待完整结果真实出现在可访问性树，再抓取 `edited-resend-final.png`。强化复跑已通过：`/tmp/nola-real-edit-visible-final.log`，1 个真实用例、0 跳过，4 个 answered seals；证据目录 `/tmp/nola-mac-real-memory.5it9F0/`。提示不包含计算答案，分支追问得到正确的 612。收尾截图 `runtime/edited-resend-final.png` 已实际检查，完整显示 REEDITED=323。

检查截图时曾捕获到末字仍在绘制渐显中的中间帧。AX 已持有完整文本，而收尾截屏早于 0.52 秒绘制动画结束；补充原生位图可见性/稳定性断言，最终截图等待有界动画窗口后采集，没有为截图修改或伪造回复文本。

正常 Debug 构建完成，使用项目配置的 Apple Development 身份，`codesign --verify --deep --strict` 通过，非 ad-hoc。构建路径 `/tmp/nola-ui-polish-mac-build/Build/Products/Debug/Nola.app`，日志 `/tmp/nola-mac-edit-signed-build.log`，签名核验 `/tmp/nola-mac-edit-signed-verification.json`。没有启动该构建去改变用户数据库。

最终格式检查、凭据扫描和版本基线 6 项通过；没有把 86 项回归或截图测试当成真实联网验收，以上真实回执分别保留。

补充绘制回归：首次完整绘制套件有 1 个数学公式前缀用例的 2 条逐像素断言失败。其初始参考图在 430 ms 采集，早于已配置的 520 ms 初始渐显时长；改为按动画时长加明确余量采集参考图，原有“前缀像素不变”“新增字形可见”等断言全部保留。最终结果见绘制专项日志，不把该轮失败隐藏为通过。

最终绘制专项 8 项全部通过，0 跳过，日志 `/tmp/nola-mac-edit-visual-verified.log`、结果 `/tmp/nola-mac-edit-visual-verified.xcresult`。包含最后单个字形实际可见、动画结束后位图稳定、已有前缀不重画、带数学公式的追加、减少动态效果及实时/历史渲染一致性。
