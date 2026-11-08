> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Contributing to Nola

Thanks for your interest in contributing.

## Development setup

1. Fork and clone the repository.
2. Start local development services:

```bash
./dev.sh
```

3. Run checks before opening a PR:

```bash
cd backend && npm run typecheck
cd backend && python -m pytest -q
cd core && cargo test --quiet
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
