# Rust 构建性能

更新：2026-11-14。目标是缩短稳定工具链下的开发反馈时间，并限制磁盘和内存占用。
不改变 Nola 1.0.0 的协议、ABI、数据库格式、panic 策略或 Release 优化级别。

## 当前策略

- dev/test 的 workspace crate 显式使用 256 个 codegen units。关闭 incremental 时，
  Cargo 原本会使用 16 个；第三方依赖仍固定为 16，避免复制整套依赖编译产物。
- 产品构建及 `test_rust.sh` 共用资源预算：最多 4 个 Cargo job，每 4 GiB 内存一个；
  内存未知时最多 2 个，最低 1 个。Linux 同时读取 cgroup 内存限制。
- 优先保留调用者的 `CARGO_BUILD_JOBS`，Cargo 的 `-j` 仍优先。可用
  `NOLA_RUST_BUILD_JOBS=2` 限制并发；`NOLA_RUST_BUILD_JOBS=cargo` 交回 Cargo 本身。
  直接运行 Cargo 不经过辅助脚本，需要按机器设置 `-j` 或本地配置。
- incremental 继续关闭；Release 与移动端优化配置沿用原值。增加并行度不新增 profile，
  不关闭运行时检查，也不复制多份 SDK 缓存。
- Mac Core 和内嵌 runtime 的依赖清单都批量转义路径，避免为每个文件启动 sed。
  Cargo 仍核验真实输入，未变化的静态库继续保留修改时间。

## 社区方案核对

| 方案 | 采用方式和边界 |
| --- | --- |
| [Cargo 构建性能指南](https://doc.rust-lang.org/cargo/guide/build-performance.html) / [profile](https://doc.rust-lang.org/cargo/reference/profiles.html) | 实测并采用有界并发与 workspace codegen units；保留 debug=0 和依赖产物复用。 |
| [mr-boxington](https://mr-boxington.jdx.dev/limits) | 延续现有接入；最终 staticlib 不能共享缓存，不能把它的重新编译误报成缓存故障。构建脚本不安装工具或改全局设置。 |
| [sccache](https://github.com/mozilla/sccache/blob/v0.19.0/src/compiler/rust.rs) | 原生解析器明确接受 rlib/staticlib。Apple/Android 静态库入口优先使用已安装的 sccache；普通 Host/CLI 保留 mbx。不叠加 wrapper，工具缺失时 auto 回退。 |
| LLD / [mold](https://github.com/rui314/mold) / [wild](https://github.com/wild-linker/wild) | 按目标平台评估；Linux 链接器参数不能套到 Apple SDK，也不能消除 staticlib 的代码生成。当前 Apple 链接器保持原配置。 |
| Cranelift、并行前端、workspace feature-unification | Cargo 当前指南中的这些配置需要 nightly/不稳定功能；当前发布路径继续使用 stable。 |
| [cargo-hakari](https://docs.rs/cargo-hakari/latest/cargo_hakari/) | 可统一 features、减少组合重复；其 workspace-hack 会改变依赖特性并集，尚未用于本项目的 Apple/Android/Windows 多目标图。当前保留真实 feature 隔离。 |

## 隔离实测

机器：Apple Silicon，10 核、16 GiB；Rust/Cargo 1.99.0，`nola_application --lib`，
opt-level=0、debug=0、incremental=false、无 compiler wrapper。先下载依赖，再离线计时。
每种配置使用全新临时 target；两轮反向顺序复测，另补 16 units / 4 jobs 对照。

| Workspace units / jobs | 冷构建（秒） | 保留依赖、重编本 crate（秒） | target 占用（MiB） |
| --- | --- | --- | --- |
| 16 / 2 | 38.48–42.64 | 9.58 | 741 |
| 256 / 2 | 37.44–38.75 | 9.53 | 747 |
| 16 / 4 | 32.79 | 8.90 | 741 |
| 256 / 4 | 29.55–30.61 | 9.00 | 747 |

该样本冷构建缩短约 23–28%，单 crate 重编约 6%，target 占用增加约 0.8%。
热构建约 0.27–0.37 秒，重新编译产物为 0。主要收益来自依赖并行，不能把冷构建收益
当作每次编辑、全部客户端或移动端 Release 的收益。此表是实际墙钟时间，不使用缓存工具的估算节省值。

```sh
# 下载不计入下面的计时。可换成本机实际 target。
cargo fetch --locked --target aarch64-apple-darwin
python3 scripts/benchmark_rust_build.py --package nola_application \
  --output .artifacts/rust-build-comparison
```

基准保留 JSON 和 Cargo timing HTML，依次测冷构建、热构建和只清理本 crate 后重编。
只清理本轮创建的临时 target，不清空用户 target；成功或失败后均回收临时目录。


## 最终静态库缓存验证

本机单独安装官方 sccache v0.19.0，并在安装前核对 GitHub release asset 的 SHA-256。
该安装步骤不在构建脚本内。缓存默认限制为 4 GiB，可显式覆盖。

对真实 `nola_mobile_bindings` 的 Mac ARM64 dev 静态库，保留依赖，只清理本 crate：
强制重新编译整脚本 5.15 秒，缓存恢复 1.51–1.87 秒；Cargo 阶段分别 4.38 / 0.63–0.73 秒。
sccache 统计新增一次 Rust cache hit；恢复后的 `.a` 与先前产物逐字节相同，复制目标的
修改时间保持不变。此结果仅证明该静态库重建的缓存命中，不代表 iOS/Android Release 已测得同等加速。

`NOLA_RUST_BUILD_CACHE=sccache` 可显式选择；`mbx` / `off` 可覆盖静态库的 auto 选择。
显式 `RUSTC_WRAPPER` / `RUSTC_WORKSPACE_WRAPPER` 包括空值，仍由调用者控制。
同时启动多个完整客户端构建时，可以显式降低 jobs，避免各进程的并行度累加。
