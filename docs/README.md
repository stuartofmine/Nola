# Nola 文档

公开文档按**用途**与**平台**组织。设计合同说明应有行为，源码映射说明当前接线，验收范围说明实际测过什么；三者不能互相替代。

| 用途 | 入口 | 内容 |
| --- | --- | --- |
| 软件架构 | [架构目录](architecture/README.md) | 总体设计、所有权、接口、15 个模块与恢复契约 |
| 跨平台交互 | [设计目录](design/README.md) | 输入、消息、检查器、权限、宿主机、主题与动效 |
| 平台接入 | [平台目录](platforms/README.md) | macOS、iOS/iPadOS、Android、Web、Windows、CLI，以及 Apple 共用设计 |
| 可操作原型 | [原型入口](design/prototype/README.md) | 68 场景、220 状态、覆盖清单与浏览器回放 |
| 开发与构建 | [工程入口](engineering/README.md) | 环境、测试配置、构建缓存与跨语言规范 |
| 验收与质量 | [质量入口](quality/README.md) | 证据分层、真实 Host 长任务与原型回放的范围和限制 |

重新发布的 1.0.0 是首个受支持基线，Nola 自有协议、ABI、数据库格式与 epoch 从 1 开始；未知格式拒绝且不自动删除或重建数据。外部标准协议沿用自身版本。详见[版本兼容策略](architecture/总体设计/VERSION_COMPATIBILITY.md)。

## 本地工作记录

`Doc/当前任务/` 与 `Doc/历史任务/` 只保留在维护者本机，已停止 Git 跟踪；不作为公开文档依赖。公开文档不链接这些未分发文件。已落地的长期契约放入架构或设计，公开验收范围放入 quality，任务流水与原始历史仍留本地。

新增文档按上述用途归类；平台特有内容进入对应平台目录，共享规则只维护一份。`docs/design/prototype/BUSINESS-SCENARIOS.json` 是原型场景清单，不是生产执行或授权目录。不要在此提交凭据、数据库、日志、临时截图、构建缓存或任务历史。

检查入口：`python3 scripts/check_docs.py`。
