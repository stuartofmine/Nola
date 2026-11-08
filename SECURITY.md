> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Security Policy

## Supported versions

Security fixes are applied to the default branch and the latest release line.

| Version | Supported |
|---|---|
| Latest `main` | ✅ |
| Older branches | ❌ |

## Reporting a vulnerability

Please do **not** open public issues for security vulnerabilities.

Use GitHub's private vulnerability reporting (Security Advisories) for this repository.  
If private reporting is unavailable, contact maintainers through private GitHub channels.

When reporting, include:

1. Affected component and version/commit
2. Reproduction steps or proof of concept
3. Impact assessment
4. Suggested mitigation (if available)

We will acknowledge reports as soon as possible and coordinate disclosure after a fix is prepared.
