> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

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
