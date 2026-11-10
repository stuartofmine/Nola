# 覆盖表证据精度复核（替代首版404/223口径）

首版汇总存在错误：只要有pass就汇总pass，吞掉了实际执行blocked，并有更多菜单证据引用到同目的地其他入口。旧结论撤回，以本版为准。

规则：源码声明不是独立可见控件数；同一声明各实例/阶段分开保留。真实执行blocked不能被取消、禁用态、组件样例或导航入口pass覆盖。所有mixed原记录保留在operation_evidence与precision_review；environment_results明确区分组件样例与隔离后端。已有真实证据的误blocked经同控件审查才修正。pass仍仅表示记录的动作/状态，不代表全部可能条件。

重新计算630条：396 pass / 230 blocked / 4 notapplicable；0未映射。

|ID|修正后|具体说明|
|---|---|---|
|CONTROL-0041|blocked|侧栏截断：禁用检查/取消通过；确认执行409，仍blocked。|
|CONTROL-0071|blocked|消息截断：取消/Escape通过；真实提交409缺显式模型端点，仍blocked。|
|CONTROL-0171|blocked|同一错误页Retry已六组合实际点击，读取恢复通过；PDF内嵌渲染仍blocked，因此综合保持blocked。|
|CONTROL-0236|pass|实际生产Markdown正文链接，人工官方导入记录；六组合本地弹出页面通过。|
|CONTROL-0237|pass|实际生产Markdown图片链接，同上；只访问本机页面。|
|CONTROL-0238|pass|代码头复制按钮六组合实际点击；完整剪贴板文本与code文本严格相等。|
|CONTROL-0246|pass|memory开关成功PUT后Reload读回，恢复后再读回。|
|CONTROL-0346|pass|retrieval开关成功PUT后Reload读回，恢复后再读回。|
|CONTROL-0424|pass|实际更多菜单上下文按钮六组合点击，不用overview快捷入口代替。|
|CONTROL-0425|pass|实际更多菜单执行记录按钮六组合点击，不用overview快捷入口代替。|
|CONTROL-0462|pass|实际Dashboard View all·0链接点击；明确修复原错误blocked。|
|CONTROL-0463|pass|UI创建禁用待批准任务，点击对应Dashboard任务行；返回计划并UI清理。|
|CONTROL-0545|pass|实际模型上移，核对非空标题顺序；Save后整页Reload核对。|
|CONTROL-0546|pass|实际模型下移，核对非空标题顺序；Save后整页Reload核对。|
|CONTROL-0600|blocked|Today：真实pending/queued条目未执行；组件样例点击通过单列，真实后端blocked。|
|CONTROL-0601|blocked|Today：真实pending/queued条目未执行；组件样例点击通过单列，真实后端blocked。|
|CONTROL-0602|blocked|Today：真实pending/queued条目未执行；组件样例点击通过单列，真实后端blocked。|
|CONTROL-0603|blocked|Today：真实pending/queued条目未执行；组件样例点击通过单列，真实后端blocked。|
|CONTROL-0604|blocked|Today：真实pending/queued条目未执行；组件样例点击通过单列，真实后端blocked。|

额外复核：桌面主顶栏、手机更多内工具栏、已开面板工具栏按实际按钮逐一点击（50操作）；更多中的会话设置/参与者同样补点；消息选择开关移除下游截断错误污染；知识三个标签和两个编辑框补直接点击/真实输入保存；PDF原生filechooser入口实点上传后清理；桌面侧栏新会话实点并清理。所有生产配置补测均恢复，没有新模型/外部调用。

各组coverage-review.json记录逐控件证据限制和补测；根coverage.html/JSON/CSV是综合表。STATUS-CORRECTIONS.json比较首版状态，REQUESTED-ID-CORRECTIONS.json列出父复核指定ID。历史脚本选择器/时序失败保留为调试证据，不算产品缺陷。

生产修复提交：2d7a7a9f437c2e8dbd4f724bd0338daf71dc270e。完整证据在独立审计包Nola-Web-Full-Control-Audit.zip；没有新增测试套件、push或部署。
