> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# iOS provider 释放回归（2026-09-28）

`/tmp/nola-ios-generation-fidelity-20260928.xcresult` 实际模拟器执行40项，34通过、6崩溃。
失败包括新参数JSON测试的成功路径和拒绝路径，以及两项原有 OpenAIProvider 测试，
因此不是单纯 XCTAssertThrowsError 的错误桥接问题。

导出的6份 crash report 均显示同一链路：

`malloc invalid free → TaskLocal::StopLookupScope → swift_task_deinitOnExecutorImpl → swift_task_deinitOnExecutorMainActorBackDeploy → OpenAIProvider.__deallocating_deinit`。

临时 provider 在同步测试栈释放时进入 SDK 的 MainActor isolated-deinit 回部署路径。
现有部分测试静态保留 provider，因此没有执行相同释放路径；本修复不扩大这种保留，
也不删除拒绝断言或吞掉错误。结论只限这次 SDK/运行时组合下已捕获的析构路径，
不泛化为全部 actor 或所有错误桥接均存在缺陷。

OpenAIProvider 没有原自定义 deinit；其存储仅为 config 值、Logger.shared 引用、
URLSession 引用，没有需要在 MainActor 上完成的 owned UI teardown。新增明确
`nonisolated deinit {}`，避免无必要的 executor 析构调度；未修改 NolaError。
原临时对象的真实释放路径继续由测试覆盖。

修复后同一组40项实际模拟器复跑82995 exit0，xcresult确认40通过、0失败、0跳过。
新参数4项、旧provider18项、canonical8项、exact5项、prepared2项、local3项均包含。
结果：`/tmp/nola-ios-generation-fidelity-fixed-20260928.xcresult`。原6项崩溃全部消失，
测试未改为长期保留实例；该验收不等于真实远程模型端到端调用已验证。
