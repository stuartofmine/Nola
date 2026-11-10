# 验收与质量

公开文档维护长期有效的验收方法、范围与限制。任务流水和历史原始记录保留本地，不作为仓库链接或发布依赖。

- [当前已验证范围与缺口](VALIDATION_SCOPE.md)：真实 Host 链路与合成原型分别记账。
- [测试架构](../architecture/模块/15-观测与验收/TESTING_ARCHITECTURE.md)、[真实模型验收方法](../architecture/模块/15-观测与验收/REAL_E2E_TESTING.md)。
- [测量合同](../architecture/模块/15-观测与验收/MEASUREMENT.md)、[隐私发布检查](../architecture/模块/15-观测与验收/privacy-release-checklist.md)。
- [原型状态回放范围](../design/prototype/docs/VALIDATION.md)、[原型场景清单](../design/prototype/docs/BUSINESS-SCENARIOS.md)。

## 验证范围与本地工作记录

纯状态/契约测试、组件/原型回放、真实客户端、真实 Host/模型与真机网络是不同层次。范围必须注明环境、源码身份、实际效果及未覆盖项；不能用后一种名称包装前一种测试。

`Doc/当前任务/` 和 `Doc/历史任务/` 不随仓库分发。公开页面提到的“本地工作记录”只表示记录保留在维护者机器，不能作为读者可打开的证据链接；不得为了补链接重新上传原始日志、数据库或任务历史。可复现源码、正式契约与脱敏汇总才进入公开目录。
