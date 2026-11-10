> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Contributing to Nola

Thanks for your interest in contributing.

## Development setup

1. Fork and clone the repository.
2. Start local development services:

```bash
npm --prefix web/frontend ci
npm --prefix web/frontend run build
cargo build --locked -p nola_cli -p nola_service --bins
./target/debug/nola web --host 127.0.0.1 --port 5001 --web-root "$PWD/apps/web"
```

3. Run checks before opening a PR:

```bash
cargo test --locked -p nola_domain -p nola_protocol -p nola_store -p nola_update_host
npm --prefix web/frontend run check
npm --prefix web/frontend test
```

## Pull request guidelines

1. Keep changes focused and atomic.
2. Add or update documentation for user-facing behavior changes.
3. Include tests for behavior changes when possible.
4. Use clear commit messages (recommended: Conventional Commits).
5. Ensure CI is green before requesting review.

## Coding expectations

- Follow existing code style and naming conventions.
- Prefer explicit error handling over silent fallbacks.
- Avoid unrelated refactors in the same PR.

## Reporting bugs and proposing features

- Use GitHub Issues and the provided templates.
- Include reproducible steps, expected behavior, and actual behavior.
