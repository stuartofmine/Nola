> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# V2 Apple 流式 UI 性能初步优化：2026-09-10

本轮针对 `ARCHITECTURE_V2.md` §9.3、§16 和 §20.4，检查 iOS/macOS 的长会话窗口、
Markdown 更新、流式 reveal 和自动滚动。修改只影响展示缓存和调度，不改变规范消息内容、
Projection 身份或历史分页契约。

## iOS

- `ChatStreamingMarkdownBuffer` 现在缓存稳定 block，只将上一次 mutable tail 与新追加文本
  重新分块。View 的旧 block 计数和 body 渲染直接读取缓存，避免每个语义 delta 对不断增长的
  完整回复重复执行三次 Markdown partition。
- 非前缀更新仍被拒绝，完成事件仍以规范全文结算；增量算法的测试在每个中英文、inline 与
  table 片段后和完整 partition 逐项比较，并检查稳定前缀身份不变。
- 自动跟随的尾随窗口从 32 次 × 110 ms 缩短为覆盖 640 ms reveal 的 7 次 × 110 ms。
  开启 Reduced Motion 时不再维持约 3.5 秒的滚动循环，只跨两个 16 ms 布局帧校正。
  用户向上滚动和任务取消仍会立即停止跟随。

## macOS

- 聊天时间线已有 20 条可见窗口、字节预算及有界两帧滚动校正，本轮保留这些行为。
- 完成态 `MacMarkdownView` 的顶层 Markdown/code/mermaid partition 增加 4 MiB、128 项缓存，
  以完整 source revision 为键。后续 sibling 流式更新导致父 View 重算时，稳定旧 block 不再
  重复扫描代码围栏。仍在增长的 streaming source 绕过缓存，避免每个前缀污染缓存。

## 验证边界

- 修改文件通过 `swiftc -frontend -parse` 和 `git diff --check`。
- iOS focused `xcodebuild` 首次完成源码编译时发现并修复一个缺失 `return`；随后模拟器测试
  进入 `simctl diagnose` 且没有生成有效 xcresult，已主动终止。
- 第二次仅 `build-for-testing` 在 6 分钟后仍持续进行全工程依赖构建并与其他并行任务竞争，
  按有界测试要求终止，没有将其记为通过。新增 XCTest 仍需在无并行 Xcode/Cargo 构建时执行。
- 本轮没有宣称完成真实 10M 字符、多窗口帧时间或 RSS 验收。
