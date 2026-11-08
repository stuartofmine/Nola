> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 流式正文规模验收

首测日期：2026-09-28；状态更新：2026-10-02。这是分层证据记录，不是全链路线性
性能已达标的声明。累计已实现链路及当前验收见[整体改造第8节](MODULAR_REDESIGN_2026-09-28.md)。

## 分层与判据

1. Store：真实磁盘SQLite，每次32KiB生产`commit_transition`，分别写2/8/32MiB。
   写入内容带Unicode及块序号；完成后冷重开，用有界range读取并校验全文SHA。
   记录Append、Close、100次tail/range耗时，SQLite缓存页写入/缺失计数及DB/WAL大小。
2. Application：身份映射和诊断预览不能先克隆累计正文再截断；确认生产调用者接入，
   测试借用正文、canonical别名、错误owner拒绝、终态及原文不变。
3. Core/端到端：检查checkpoint序列化、累计Part事件、传输与UI投影。
   目前仍有累计成本，不能用第1层固定checkpoint测量代替这一层。

SQLite缓存页计数不是操作系统物理I/O、fsync次数或设备写放大。DB/WAL文件大小是
测量时占用，不是累计写入量。debug构建耗时不能直接当产品发布性能。
tail/range用相同访问次数比较，不把读取总正文的SHA验证混入有界读取成本。

## 可复现入口

```sh
CARGO_INCREMENTAL=0 cargo test -p nola_store --lib production_stream_2_8_32_mib_disk_acceptance -- --ignored --nocapture --test-threads=1
```

测试位于`core/nola-store/src/canonical_stream_performance_tests.rs`，默认ignored，
每个规模使用独立TempDir并在下一规模前释放，仅清理本测试创建的fixture。
不得清理用户数据库或生产凭据为测量腾空间。

## 必须保留的未完成项

- Core checkpoint仍保存累计状态；当前生产事件也包含累计Part。
- Part诊断已独立拆分：通用借用serializer先限字符串/键/格式化与深度，再由writer
  校验整条事件实际编码字节**<12,000**。此前13,334字节回归已闭合，canonical仍保留
  完整structured result；六crate lib通过，见`/tmp/nola-public-semantics-libs-r6-20261001.log`。
  这是Part诊断编码的有界性，不覆盖非Part事件、自定义Serialize内部工作或Core累计状态。
- 远端累计snapshot仍有8MiB协议上限；必须按所有backing计算并明确拒绝超限，
  不能把窗口截断伪装完整正文。最终替换为已提交journal/range传输另行验收。
- Mac/CLI实际呈现、真实模型及全Store物理配额/旧generation回收尚待验证。
- 10-02完整publication-set和多frame planner已实现，canonical public owner映射借用
  大Text/JSON字段，不clone累计Part；新增指针/帧预算/真实SQLite冷重开已通过六crate lib。
  这证明具体边界的有界性与完整性，不等于完成全Core或端到端规模测量。

## 首次实测（debug，2026-09-28）

上述命令session35063 exit0，日志`/tmp/nola-stream-scale-20260928.log`。
三个规模均通过真实生产提交、Close及冷重开全文SHA校验。
这是当批历史记录；该旧临时日志目前已不在，10-02的新实测单独列于下节。

| 正文字节规模 | Append次数 | Append耗时ms | SQLite缓存页写入 | Close耗时ms | 100次tail+range耗时ms | DB字节 | WAL字节 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2MiB | 64 | 335.258 | 2,274 | 174.183 | 194.852 | 5,013,504 | 4,247,752 |
| 8MiB | 256 | 1,245.725 | 9,217 | 688.810 | 194.552 | 18,341,888 | 4,260,112 |
| 32MiB | 1,024 | 4,976.776 | 37,417 | 2,757.351 | 194.322 | 69,271,552 | 4,260,112 |

SQLite page_size=4096、journal_mode=WAL。固定次数tail/range均为零缓存页缺失和写入；
这是热缓存结果，不是冷磁盘读取延迟。32MiB相对2MiB，数据量16倍、追加耗时约14.85倍、
缓存页写入约16.45倍；支持本次Store链路近似线性增长，不证明所有负载或Core层复杂度。
测试没有硬编码机器耗时阈值，完整性断言与性能观测分开。

## 当前源码重验（debug，2026-10-02，schema33）

使用上方同一显式ignored命令，日志
`/tmp/nola-journal-publication-set-scale-r1-20261002.log`：**1通过、0失败、exit0**。
2/8/32MiB均经过实际`commit_transition`、Close、冷重开及完整SHA校验；每个规模
使用独立TempDir。该单项是三个规模的参数化验收，不另外累计成三项新增测试。

| 正文字节规模 | Append次数 | Append耗时ms | SQLite缓存页写入/缺失 | Close耗时ms | 100次tail+range耗时ms | 冷读全文SHA耗时ms | DB字节 | WAL字节 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2MiB | 64 | 311.819 | 2,274 / 3 | 176.523 | 198.008 | 90.786 | 5,038,080 | 4,247,752 |
| 8MiB | 256 | 1,180.237 | 9,217 / 5 | 700.408 | 197.673 | 362.711 | 18,366,464 | 4,260,112 |
| 32MiB | 1,024 | 4,719.572 | 37,417 / 264 | 2,794.707 | 197.577 | 1,451.160 | 69,296,128 | 4,260,112 |

SQLite page_size=4096、WAL；本次每组100次tail/range各有1次缓存页缺失、0写入，
不是旧批次的零缺失。32MiB相对2MiB的Append耗时约15.14倍、缓存页写入约16.45倍，
仅支持固定checkpoint的Store链路近似线性；不据不同日期的debug耗时宣称产品提速。
本次没有运行公开journal传输、Core大checkpoint或UI端到端性能测试。

## 旧历史range的扫描缺口（2026-10-02）

完整Store tests中的`append_messages_e2e::history_byte_window_preserves_large_messages_and_advances_without_gaps`
通过，但其7项所在suite耗时179.74s。fixture有千万字节inline text及同量JSON；冷重开
约306页读回时，每页`can_read_original → allows_product_text → extract_json_scalar_fields`
均先验证整份JSON，再扫描token并核验两遍hash，仅为确认review可披露。512-byte
输出预算限制内存，不限制扫描。SQLite inline BLOB cast还保留累计复制成本；这与
实时private chunked Text及上节直接canonical reader的规模测量不是同一读取路径。

整改要求：在真实writer/import/sync提交边界生成并绑定owner、字段generation/root及
精确版本的disclosure事实；可变字段变更必须原子失效。正文同时采用真正有界backing，
不能以最后内容缓存、相同长度或预览猜版本，也不能跳过review/owner/冲突校验。
这项生产整改尚未实施，不把本次正确性通过写成历史读取性能已闭合。

设计依据：OpenAI Docs的[流式响应事件](https://developers.openai.com/api/docs/guides/streaming-responses)
区分delta和生命周期；本地OpenCode `session/processor.ts:406`起将start、delta、end
分开处理。两者不提供Nola内部SQLite复杂度或性能保证，以上须以实测证明。
