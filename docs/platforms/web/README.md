# Web

实现位于 [web/frontend](../../../apps/web/frontend)，Svelte 界面由用户拥有的 Rust Host 提供；浏览器不拥有第二套业务 Runtime，也不在浏览器本地执行桌面工具。

- [Web 构建目录](../../../apps/web/README.md)、[Host 传输](../../architecture/模块/14-远程Host与同步/README.md)。
- [会话交互](../../design/shared/CONVERSATION_UI_DESIGN.md)、[检查器](../../design/shared/CONVERSATION_INSPECTOR.md)。
- `npm --prefix apps/web/frontend ci`，然后运行 `run check`、`test`、`run build`。启动方式见[仓库 README](../../../README.md)。

公开/本地连接使用各自认证与 CSRF/信封边界，连接成功不授予工具操作。真实浏览器 → Host 200 步、压缩、取消及冷启动追问与[原型回放](../../design/prototype/docs/VALIDATION.md)分开记账，详见[实际范围](../../quality/VALIDATION_SCOPE.md)。
