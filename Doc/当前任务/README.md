# 当前任务入口

更新：2026-11-10。

当前目标：**按现行设计契约更新各平台 UI 和交互；原型用于视觉参考与演示，未同步资产不能覆盖现行交互。**

当前先按[逐页原型对齐](UI_PAGE_ALIGNMENT_20261029.md)推进：截图对照、调整页面、连续使用并修复问题，最后由测试补充验证。范围包含 Web、iOS、macOS、Android，以及 CLI 适用的交互；不仅是主题配色。

此前实现记录见[跨平台 UI 实施](UI_PROTOTYPE_IMPLEMENTATION_20261028.md)，协议与专项证据见[验收审计](UI_PROTOTYPE_ACCEPTANCE_AUDIT_20261029.md)。这些记录不能替代当前逐页视觉和完整使用验收。失败专家新尝试见[共享设计](EXPERT_FAILED_ATTEMPT_DESIGN_20261029.md)，该设计尚未实施。

上一阶段记录见 [历史归档](../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)，不自动将旧待办并入当前任务。现行交互以[对话规范](../UI和交互设计/CONVERSATION_UI_DESIGN.md)、[检查器规范](../UI和交互设计/CONVERSATION_INSPECTOR.md)和[视觉系统](../UI和交互设计/DESIGN_SYSTEM.md)为准；原型未同步资产保留演示边界，业务权威遵守现行架构契约。

- [真实模型 UI 验收与测试清理](UI_REAL_MODEL_ACCEPTANCE_CLEANUP_20261030.md)：删除离线模拟 UI 验收，仅真实模型产品工作流计入验收。

## 最新结果与剩余验证（2026-11-10）

[本轮实现与验收总账](REMOTE_HOST_AND_PROTOTYPE_ACCEPTANCE_20261110.md)记录远端权限修复、Web 200 步长任务与两次实际压缩、Host 重启追问、Android 两分钟作业及全业务原型；原始失败与测试定位问题不混为产品缺陷。后面的日期段落保留历史范围，不继续覆盖本节。

| 工作 | 状态 | 后续范围 |
| --- | --- | --- |
| Android 远端权限详情与操作可达性 | 已修复、真实作业及回归通过 | 未覆盖所有系统权限/字号/语言组合 |
| Web → Host 200 步、重连、取消、压缩后重启追问 | 本轮通过 | Mac/iOS 原生同等链路、Windows 系统、双设备网络未验收 |
| Android → Host 120 秒作业、客户端强停后恢复 | 本轮通过，另有 90 秒收尾复跑 | 真机后台挂起、Wi-Fi/Bonjour/防火墙未验收 |
| 68 场景、220 状态全业务原型 | 合成浏览器回放通过 | 不作为生产全部功能完成证明；不发布旧外部 Site |

[场景清单](../../docs/design/nola-ui-prototype/docs/BUSINESS-SCENARIOS.md)是设计范围，[原型验证](../../docs/design/nola-ui-prototype/docs/VALIDATION.md)是演示证据；实际产品和真实模型结果只按本轮总账列出的范围记账。

## 现行文档核对

2026-11-08 已按当前工作树更新原生档位、模板位置、侧栏展开、手机导航、主题/动画以及[客户端配置架构](../软件架构设计/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。旧跨端进度流水已与现行验收门槛分离。这项文档整理不关闭本页已有 UI/真实模型验收，也不将 CLI、所有 provider 或真机覆盖标记为完成。

2026-11-08 架构专项复核已扩大到总体部署、实际依赖与 15 个模块入口，见[源码核对及真实差距](../软件架构设计/总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。本轮随后继续修复了 memory owner 的领域记录、Application/Store/Host 接线，移除了 bootstrap/Service/Mac 旧 provider 自动恢复链路，并补齐 Mac 计划保存的 revision 前置条件；详见[记忆所有权合同](../软件架构设计/模块/12-记忆数据源与自动化/MEMORY_SCOPE_V2.md)。类型/依赖边界、原生局部 owner 选择界面与真实模型/真机验收仍按各自证据记录，不由这些修复推定完成。

本轮代码修复与验收范围见[未贯通链路修复记录](ARCHITECTURE_WIRING_REPAIR_20261108.md)。

2026-11-08 已补充 [CLI 真实模型长任务验收](CLI_REAL_LONG_TASK_ACCEPTANCE_20261108.md)：117 次真实请求，六类最终场景通过，覆盖长链、冷启动、两次实际压缩、记忆 owner、权限等待与后台任务；PTY/图片粘贴/真机 UI 仍待独立验收。

2026-11-08 已补齐 [跨平台真实模型长任务验收](CROSS_PLATFORM_REAL_LONG_TASK_ACCEPTANCE_20261108.md)：Mac、Web、iOS Simulator 与 Android AVD 的选定场景最终通过，并修复 Mac 空推理档位、Android 协议版本和取消收尾期间发送过早启用的问题。模型配置表单的自动化耗时异常、物理手机和未列出的 UI/故障组合仍保留验收边界。

2026-11-08 [跨平台人机工程优化](CROSS_PLATFORM_ERGONOMICS_20261108.md)：移动端消息动作密度与命中区域、复制反馈生命周期、Web 搜索焦点和滚动跟随、CLI Unicode 编辑、Windows 菜单与窗口恢复；测试覆盖与剩余边界独立记录。

2026-11-08 新增[软件更新机制实现与验收](SOFTWARE_UPDATE_ACCEPTANCE_20261108.md)：可信发布合同、签名/完整性、跨端入口和版本发布工具。线上发布源、公钥与真实签名安装包仍须由维护者配置；不据此宣称线上更新已开通。
