# GitHub 发布与软件更新

唯一公开项目：[stuartofmine/Nola](https://github.com/stuartofmine/Nola)。源码在 `main`，完整安装包放在 GitHub Releases，签名更新信息位于 `release/feed.json`，客户端匿名下载。

`update-source.json` 是随程序编译的当前 version/build、渠道、公开 feed URL 与 Ed25519 公钥。私钥只保存在发布者本机或 GitHub Actions Secret，不进入源码和安装包。更新不依赖 Cloudflare；项目的独立账户/推送后端仍可使用 Cloudflare。

首次发布以真实完成构建和验收的平台为准；未提供目标包的端显示“暂无更新包”，不能用其他平台的包代替。

## 当前版本与公开源码

```sh
python3 scripts/releases/prepare_version.py --check
python3 scripts/releases/prepare_version.py --version 1.0.1 --build 2 --dry-run
```

确认版本变更后去掉 dry-run。build 全平台单调增长；Apple、Android、Windows 和源身份必须一致，Cargo 内部 crate 版本不代表 App 发布版本。

从现有私有工作树首次公开时，导出当前源码并创建新仓库历史：

```sh
python3 scripts/releases/export_public.py --destination /path/to/new-public-checkout
```

导出不复制 `.git`、签名材料、本地数据或构建目录；即使敏感文件原来已被私有仓库跟踪，也不会进入公开快照。已人工核对的测试凭据只按路径和内容指纹允许，不放宽新凭据检测。原私有目录与历史不删除。

## 制作完整发行包

先生成前端资源并构建匹配的 executables，再打包。macOS ARM64 CLI 的包同时包含 nola、nola-agent-host、nola-service、Web 资源和托管浏览器 runtime；保留目录结构。

```sh
npm --prefix web/frontend ci
npm --prefix web/frontend run build
cargo build --release --locked -p nola_cli -p nola_service --bins
python3 scripts/releases/package_cli.py --output /staging/Nola-cli-macos-aarch64-1.0.0.zip \
  --browser-runtime /staging/managed-browser-runtime --node-license /staging/Node-LICENSE.txt
```

平台签名/公证必须在计算包 digest 前完成；开发预览若未进行 Developer ID 签名与公证，要在发布说明明确标注。Android 不能用 Debug 签名冒充 Release；商店包使用 `-PnolaDistribution=store`，禁止请求 APK 安装权限。没有经过对应系统构建和安装验收的平台不发布安装包。

## 签名并发布

使用仓库外、仅本人可读的 Ed25519 私钥；工具要求 OpenSSL 3：

```sh
python3 scripts/releases/sign_update_feed.py public-key \
  --openssl /path/to/openssl-3 --private-key /secure/path/signing.pem
```

只把公钥写入 `update-source.json`。对当前实际发布目标填写 manifest；包 URL 使用明确 tag：`https://github.com/stuartofmine/Nola/releases/download/vVERSION/FILENAME`。说明为纯文本，时间为 UTC Unix 秒，`store_schema` / `store_epoch` 必须匹配当前源码常量。示例不是可直接上传的正式信息。

```sh
python3 scripts/releases/sign_update_feed.py sign \
  --openssl /path/to/openssl-3 --private-key /secure/path/signing.pem \
  --source release/update-source.json --manifest /staging/manifest.json \
  --artifact-dir /staging/packages --output /staging/feed.json
cargo run --locked -p nola_update_host --example verify_feed -- \
  release/update-source.json /staging/feed.json cli-macos-aarch64
```

在 Releases 中先建立 draft、上传真实完整包并确认 GitHub 返回的 SHA-256 与签名 manifest 一致，再发布该 draft，最后将已验证的 feed 提交到公开 `main`。不要上传测试夹具、密钥、数据库或仅替换一个可执行文件。

客户端允许最多五跳 HTTPS；仅允许同源及 GitHub → 官方 `release-assets.githubusercontent.com` 跳转，拒绝降级、嵌入凭据、其他来源。无 GitHub Token 内置到客户端；每个安装包仍须匹配已签名的大小和 SHA-256。

## 启用与续签

完成线上包和 feed 发布后，再检查实际 HTTPS 链路：

```sh
python3 scripts/releases/activate_source.py --source release/update-source.json \
  --target cli-macos-aarch64 --dry-run
```

该守卫先调用当前 Rust 协议验证器，再流式核对线上包。失败不写源，不能把 404 或错误签名当成“最新”。源是构建时数据，已安装的旧二进制不会被脚本偷偷改写。

feed 有效期为 28 天。`refresh-update-feed.yml` 每周验证原签名并仅更新时间戳，不改变版本、目标、说明或包摘要，生成新签名并经 Rust 再验证后提交。需要仓库 Actions Secret **NOLA_UPDATE_SIGNING_KEY**；私钥通过 GitHub 加密 Secret 保存，不放进 workflow 文本或日志。没有 Secret 时明确跳过续签，首次本机签名发布仍可用，但维护者必须在到期前手动续签。

```sh
python3 scripts/releases/refresh_feed.py --openssl /path/to/openssl-3 \
  --private-key /secure/path/signing.pem
```

私有源码仓库历史不随首次公开提交发布；后续在公开仓库正常审阅和提交。真实模型验收工作流仍仅手动触发，不因公开仓库 push 自动产生模型费用。
