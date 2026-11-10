> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Nola Rust、Swift 与 Kotlin 代码规范

状态：Normative

适用范围：`core/`、`apps/cli/`、`apps/ios/`、`apps/macos/`、`apps/android/` 及对应测试

最后核对：2026-09-12

本文把语言官方规范收敛为 Nola 可执行的仓库规则。它不复制外部规范全文；链接用于解释语言惯例，本文中的“必须”和“禁止”用于代码审查。

## 1. 规范来源与优先级

| 语言/平台 | 权威来源 | 本仓库采用范围 |
| --- | --- | --- |
| Rust | [The Rust Style Guide](https://doc.rust-lang.org/style-guide/) | 格式、缩进和默认 `rustfmt` 风格 |
| Rust | [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) | 公共 API 的命名、类型安全、文档和可预测性；本项目不采用旧版本兼容要求 |
| Rust | [`cargo fmt`](https://doc.rust-lang.org/cargo/commands/cargo-fmt.html) 与 [`cargo clippy`](https://doc.rust-lang.org/cargo/commands/cargo-clippy.html) | 本地及 CI 检查入口 |
| Swift | [Swift API Design Guidelines](https://www.swift.org/documentation/api-design-guidelines/) | API 调用点、命名、参数标签、文档和 mutating/nonmutating 对称性 |
| Swift | [`swift-format`](https://github.com/swiftlang/swift-format/blob/main/README.md) | 可重复的机械格式检查；它本身不定义唯一的官方 Swift 风格 |
| Kotlin | [Kotlin Coding Conventions](https://kotlinlang.org/docs/coding-conventions.html) | Kotlin 语言惯例、文件组织、命名和不可变性 |
| Android | [Android Kotlin style guide](https://developer.android.com/kotlin/style-guide) | `apps/android/` 的格式冲突裁决：四空格、100 列、import 和换行规则 |
| Android | [Android architecture recommendations](https://developer.android.com/topic/architecture/recommendations) | Compose、ViewModel、repository、UDF 和生命周期边界 |

上述页面均于 2026-09-12 访问。优先级如下：

1. 数据安全、当前唯一协议和 [现行架构所有权](../总体设计/架构决策与所有权.md)边界。
2. 本文明确写出的 Nola 规则。
3. 对应语言或平台的官方规范。
4. 当前文件局部风格，仅用于本文没有规定的部分。

Android Kotlin 的格式以 Android style guide 为准，Kotlin Coding Conventions 补充语言惯例。Swift API Design Guidelines 是 API 规则，不是完整排版规范；在仓库提交统一 `.swift-format` 配置前，`swift-format` 只用于检查改动文件，不可用工具默认值批量重排全仓库。

## 2. 所有语言共同遵守的模块契约

文件拆分必须改变职责和可见性，不能只把同一个巨型对象分散成多个 extension 或同义包装。每个 feature/bounded context 都要能回答以下问题：

| 契约项 | 必须说明的内容 |
| --- | --- |
| 输入 | typed command、event、request、ID 或依赖端口；哪些输入允许为空或重复 |
| 输出 | typed value、state、event、stream 或 receipt；输出的顺序与一致性 |
| 状态 | 唯一 owner、生命周期、持久化位置、容量上限和恢复来源 |
| 错误 | 稳定错误类别、可重试性、面向用户的转换边界 |
| 副作用 | 数据库、网络、文件、进程、权限请求及其 transaction owner |
| 并发 | actor/lock/scope、取消路径、终止状态、顺序与幂等键 |

接口设计规则：

- 接口围绕调用者需要的能力设计；不能把数据库、HTTP payload、UI 控件或巨型 service 的全部方法原样暴露。
- 一个状态只有一个写入 owner。其他模块通过 command/event 调用它，通过 snapshot/projection/stream 读取它。
- 构造器建立必要不变量。不能要求调用者按隐藏顺序调用若干 setter 才得到有效对象。
- 边界输入和输出使用领域类型。跨异步、持久化、进程或平台边界的 conversation/run/task/attempt/part ID 必须 typed 或由明确类型封装。
- 纯规则放在 policy/reducer/kernel；IO 放在 adapter/repository/driver；生命周期协调放在 coordinator/supervisor；UI 只渲染 projection 并发送 action。
- 错误属于接口。控制流不能解析本地化文案、provider 原始字符串或模型自然语言。
- 注释解释所有权、不变量、复杂度和取舍，不复述语句。
- 测试验证通用契约和失败边界，不按 benchmark 题目、语言关键词、固定模型输出或特定文件名特化生产实现。

新模块或大规模拆分在对应架构文档中至少记录一行：

```text
Module: <name>
Owns: <state and lifecycle>
Input: <typed commands/events/ports>
Output: <typed values/events/projections>
Errors: <stable categories and retry contract>
Side effects: <IO and transaction owner>
```

## 3. Rust

### 3.1 格式、文件和命名

- 机械格式以 workspace 根目录执行的 `cargo fmt --all` 为准。采用默认 Rust 2021 style，不新增个人化对齐规则。
- 使用空格和四空格块缩进；默认最大行宽 100。使用 block indent，避免为了视觉对齐产生大段空格。
- type、trait、enum variant 使用 `UpperCamelCase`；函数、方法、模块、局部变量使用 `snake_case`；常量和 static 使用 `SCREAMING_SNAKE_CASE`。
- 模块和文件使用 `snake_case`。一个文件围绕一个领域职责；`mod.rs`/`lib.rs` 主要负责声明、稳定重导出和组合，不积累业务流程。
- getter 直接使用领域名，如 `status()`；转换严格区分借用 `as_`、可能分配 `to_` 和消费所有权 `into_`。

### 3.2 类型与公共 API

- 字段默认私有。仅同 crate 需要时使用最窄的 `pub(crate)`；公共重导出只来自稳定 facade。
- trait 放在需要抽象的调用侧，并按能力拆分。dispatcher 可以组合多个 family trait，但单个 handler 只依赖它实际使用的 family。
- 输入字段多、相邻参数同型或参数超过五个时，定义 request/options struct。不要依赖参数位置区分多个 `String`、`Uuid` 或整数。
- 有效失败使用 `Result<T, TypedError>`，可缺失值使用 `Option<T>`。library 层保留 source error 和 source；binary/UI 边界才转换为用户文案。
- 新的公共数据类型应实现语义允许的常见 trait，例如 `Debug`、`Clone`、`Eq`、`Hash`、`Default`、`From`/`TryFrom`。不要为满足 checklist 实现错误语义。
- 用 newtype 或 enum 排除无效状态。协议 enum 的未知值明确拒绝，不设兼容分支，不能用 wildcard 静默丢弃重要状态。
- 公共 API 文档说明错误、panic 条件、取消、side effect 和非显然复杂度；示例优先可由 doctest 验证。

### 3.3 所有权、异步和副作用

- 返回拥有的数据或生命周期清楚的借用。不能用 `Arc<Mutex<_>>` 代替模块所有权设计。
- lock 内不执行网络、文件、长计算、await 或外部 callback；多个锁必须有固定顺序。
- 业务 task 归属于 runtime supervisor 或明确 scope，并具有取消、join/终止和清理路径。禁止 detached spawn 长期持有领域状态。
- reducer/kernel 不读取环境变量、系统时钟、随机数或 IO；这些值通过输入或 port 注入。
- 生产路径不使用 `unwrap`/`expect`，除非类型或紧邻检查已经证明不可达，并写明理由。
- transaction owner 必须唯一。handler 不得绕过 application/store port 自行获得连接并开启第二事务。

### 3.4 仓库检查

从仓库根目录执行：

```bash
cargo fmt --all -- --check
cargo test --workspace --quiet
cargo clippy --workspace --all-targets -- -D warnings
```

开发中可先缩小到实际 crate，例如：

```bash
cargo test -p nola_application --quiet
cargo clippy -p nola_application --all-targets -- -D warnings
```

已有 Clippy 债务若阻塞全 workspace 检查，应在变更说明中记录具体 lint、crate 和输出。不得为通过检查新增 crate/module 级全局 `allow`；局部 `allow` 必须紧贴无法避免的位置并解释原因。

## 4. Swift

### 4.1 API、命名与文件

- 以调用点清晰为首要目标。方法名与参数标签组成自然、无歧义的表达；省略重复类型信息，但不为缩短名字牺牲含义。
- type 和 protocol 使用 `UpperCamelCase`，成员使用 `lowerCamelCase`。常见缩写作为完整单词一致处理，例如 `conversationID`、`URLRequest`、`isHTTPEnabled`。
- 有副作用的方法使用祈使动词；无副作用的方法读作名词短语。布尔成员读作断言，例如 `isEmpty`、`canRetry`、`contains(_:)`。
- mutating/nonmutating 对采用 Swift 惯例，例如 `sort()`/`sorted()`；表示能力的 protocol 使用语义名，不机械添加 `Protocol` 后缀。
- 一个主类型一个文件；紧密相关的小 value、私有 adapter 或 extension 可同文件。不能使用 `Foo+Part1.swift` 把巨型类型的同一职责隐藏在 extension 中。
- 避免无边界的 `Manager`、`Helper`、`Utils`。按职责选择 `Repository`、`Adapter`、`Reducer`、`Policy`、`Coordinator`、`Presenter`、`Scheduler` 或明确领域名。
- 对跨模块 API、非显然不变量、错误、复杂度和生命周期写 `///` 文档。非 O(1) computed property 必须标注复杂度或改成方法。

### 4.2 类型、状态与 SwiftUI

- 不可变领域值优先 `struct`，有限互斥状态优先带关联值的 `enum`，有身份的可变状态 owner 才使用 `class`/`actor`。
- service 通过小 protocol/closure port 注入。有生命周期的 service 在 App composition root 构造；`static let shared` 仅用于真正不可变且无外部资源的值。
- SwiftUI `View` 组合布局、渲染 projection、发送 action。它不解析 host 协议、不访问数据库、不拼接 canonical history，也不决定权限策略。
- `@Observable`/`ObservableObject` 只拥有一个 feature 或 scope 的状态。UI state 由 `@MainActor` 发布；较短生命周期的状态下沉到最接近使用处。
- streaming delta 经 reducer 依据稳定 part ID 更新。View 不能通过字符串拼接、扫描显示文本或延迟重绘猜测最终消息。
- 高频更新先合帧；稳定 Markdown block 使用缓存；动画尊重 Reduce Motion，并避免为完成态重新替换整个视图树。

### 4.3 Concurrency、错误与资源

- 优先 structured concurrency。每个 `Task` 必须由对象或 scope 持有，并定义 replacement、取消和 terminal cleanup。
- 跨隔离域的值满足 `Sendable`。不得用 `@unchecked Sendable` 掩盖共享可变状态；确需使用时必须在类型旁写明同步不变量并有并发测试。
- actor/main-actor hopping 在 service 边界集中处理，避免 View 中散落 `Task { @MainActor ... }`。
- 持久化、协议、权限和恢复错误转换成 typed feature state 或结构化日志；禁止空 `catch` 和静默 `try?` 丢弃必须处理的错误。
- closure 捕获长期 owner 时审查生命周期；逃逸 UI 回调通常使用 `[weak self]`。取消不是失败，不能显示为普通错误。

### 4.4 仓库检查

当前 Xcode toolchain 包含 `swift-format`，可通过 `xcrun` 定位，但仓库尚未提交
`.swift-format`。工具默认缩进与仓库四空格规则不同，因此当前不能把无配置的 strict lint
作为通过门禁。引入门禁时必须先提交并评审仓库级配置，然后使用：

```bash
xcrun --find swift-format
xcrun swift-format lint --strict --parallel --configuration .swift-format <changed-file.swift> ...
```

在此之前按当前文件四空格风格审查改动，禁止用默认配置批量格式化 `apps/ios/` 或
`apps/macos/`。格式检查不能替代编译与行为测试：

```bash
./scripts/run_client_tests.sh                  # iOS；脚本负责 simulator 选择与清理
./scripts/test_mac_layers.sh contracts         # 常规 macOS 边界测试
./scripts/test_mac_layers.sh host-e2e          # 改动宿主进程生命周期时
./scripts/test_mac_layers.sh full              # 合并前的完整 macOS 范围
```

UI 改动还需在目标平台检查首帧、streaming、完成态、滚动、Reduce Motion 和错误恢复。单元测试无法证明没有标题闪烁、重复重绘或动画卡顿。

## 5. Kotlin / Android

### 5.1 格式、文件和命名

- Android 源码使用 UTF-8、四空格缩进、K&R braces、每行一个 statement、不使用分号，列宽上限 100。
- 禁止 wildcard import。import 保持单组、ASCII 顺序；package 与 import 不为列宽强行换行。
- class/interface/object 使用 `UpperCamelCase`，函数、属性和局部变量使用 `lowerCamelCase`；真正深不可变的 `const val` 使用 `UPPER_SNAKE_CASE`。
- 单一顶层类型的文件与类型同名。多个紧密相关声明使用表达主题的 `UpperCamelCase.kt` 文件名；禁止 `Utils.kt`、`Helpers.kt`、`Common.kt` 等无职责名称。
- 文件只承载一个主题，通常不超过几百行。相关 declaration 相邻排列，使阅读顺序体现行为流程；不能按修改时间把新方法持续追加到末尾。
- 优先 `val` 和只读 collection interface；有限互斥状态使用 `sealed interface`/`enum class`，传输与 UI value 使用 `data class`。
- library/module boundary 明确 visibility 和返回类型；公开 API 使用 KDoc 说明错误、线程、取消和副作用。

### 5.2 Compose、ViewModel 与模块边界

- Compose Screen 接收 immutable state 和 callbacks，向上发送 action。它不访问 repository、HTTP、Room、DataStore 或 Agent Host。
- ViewModel 是 screen-level business state holder。它通过 constructor 注入 use case/port，不保存 `Activity`、`Context`、`Resources` 或 View 引用。
- ViewModel 对 UI 暴露只读 `StateFlow`；持续数据从 repository/data layer 流入。UI 使用 lifecycle-aware collection。
- 复杂且可复用的 UI element state 放到 plain state holder；跨配置变化的业务状态放到 ViewModel。每份 state 仍只有一个写入 owner。
- 纯转换从 ViewModel 移到 `*Policy.kt`、`*Reducer.kt`、`*Presentation.kt`；IO 和持久化通过 repository/data source；跨层复用的业务操作使用 use case。
- 不维护同一消息、trace 或 permission 的多份 mutable list。canonical state 经 projection 转为 UI state，事件只触发 owner 更新。

### 5.3 Coroutine、Flow 与错误

- coroutine 属于 `viewModelScope`、worker scope 或显式 supervisor。禁止 `GlobalScope` 和无 owner 的长期 job。
- dispatcher 通过构造器注入；测试使用 test dispatcher/virtual time，不能依赖真实 sleep。
- `CancellationException` 必须继续传播。`catch` 只捕获能够恢复或能够转换为稳定领域错误的异常。
- 冷/热 Flow 的生命周期和 replay 要在 owner 处明确；collector 消失时停止不必要工作，远程 observation 有重连上限、cursor 和 terminal 状态。
- UI error 使用稳定类型或 resource key，在 UI 边界本地化；protocol/domain 层不能生成本地化字符串。

### 5.4 仓库检查

当前 Gradle 工程有 Android Lint，没有配置 ktlint/detekt。因此现阶段可执行门禁是：

```bash
cd apps/android
./gradlew :app:lintDebug :app:testDebugUnitTest
./gradlew :app:connectedDebugAndroidTest  # 仅涉及 JNI、Compose 或设备边界时
```

Android Lint 不能完整验证 Kotlin 排版。提交前还应使用 Android Studio 的 **Kotlin style guide** 格式配置检查改动文件。若以后引入 ktlint/detekt，应先提交版本锁定和配置，再把对应 Gradle task 升级为仓库门禁，不能假设尚不存在的 `ktlintCheck` 可用。

## 6. Review checklist

- 文件名和类型名能否直接说明领域职责？文件拆分后是否实际减少了职责与可见性？
- 每个模块的输入、输出、状态、错误、副作用和并发边界是否明确？
- 新状态是否已有 owner，是否引入第二套 reducer、缓存或 canonical truth？
- 跨层依赖是否指向 port/protocol，而不是具体数据库、provider、HTTP client 或 UI framework？
- API 在调用点是否清楚；同型参数、字符串 ID 和字符串错误是否已类型化？
- task/stream/job 是否可取消、有界、可恢复，并在所有 terminal path 清理？
- 格式化、编译、单元、集成和 UI 验证是否与变更范围匹配？
- 测试是否覆盖公开契约、失败和并发顺序，而非某一道 benchmark 或固定模型措辞？
