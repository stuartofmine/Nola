> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Mobile Privacy Release Checklist

This checklist separates code-enforced privacy behavior from App Store and Play Console declarations. Store declarations must describe the production deployment, not only the open-source default.

## Code-enforced baseline

- No tracking domains or cross-app tracking behavior are configured.
- iOS Required Reason APIs are declared in `Nola/PrivacyInfo.xcprivacy`:
  - `CA92.1`: app-only preferences in `UserDefaults`.
  - `C617.1`: metadata for files inside Nola's app container.
  - `E174.1`: free-space checks before model downloads.
- Public Web Host addresses require HTTPS. HTTP is limited to local/private destinations and still uses application-layer HMAC and AES-GCM.
- Mobile location tools request approximate/coarse location only.
- Hybrid device results leave the device only after an action-specific approval and only for the paired host session.
- Access tokens are stored in Keychain or Android Keystore and are excluded from persisted model snapshots, logs, and exports.
- The user-owned CLI Host persists its own access token only in `~/.nola/cli/web/mobile-access-token`, with private file permissions (`0600` on Unix); it is not stored in chat, model, or diagnostic records.
- Remote Web login verifies that Host token once, never writes it to browser storage, and replaces it with an expiring HttpOnly/SameSite session whose server-side identifier and CSRF value are retained only as digests in memory.
- Android system cloud backup and device-transfer backup are disabled. User data leaves the app only through the explicit export action.
- Android explicit exports use a content whitelist and exclude credentials, Keystore material, secure preferences, client identifiers, and SQLite WAL/SHM files.
- Explicit exports redact structured MCP URL credentials and record the redaction count. Free-text user content is preserved, never silently rewritten, and requires a privacy warning confirmation before sharing.

## Deployment-specific data inventory

Before every production release, the service owner must answer these using the deployed backend configuration and retention policy:

- Does a Nola-operated service receive or retain email addresses or account identifiers?
- Are chat prompts, responses, attachments, memories, skills, documents, or tool outputs retained by a Nola-operated service?
- Are diagnostic logs, IP addresses, request metadata, crash reports, or performance metrics retained? For how long?
- Can support staff access any retained user content, and is access audited?
- Which subprocessors receive data, and are their SDK privacy manifests current?
- Does deletion remove server data, backups, derived indexes, and credentials within a documented period?

Do not declare local data, data sent only to a user-owned host, or data sent to a user-selected third-party model endpoint as collected by Nola unless the production operator also receives or retains it. Conversely, any Nola-operated relay, account service, sync service, or telemetry pipeline must be reflected in the store declarations.

## App Store Connect

- Generate and inspect Xcode's privacy report from the release archive.
- Reconcile the aggregated report with App Privacy answers and the deployment-specific inventory above.
- Confirm every bundled third-party SDK ships its own valid privacy manifest where Apple requires one.
- Confirm purpose strings describe local use and the explicit paired-host egress exception accurately.
- Verify account deletion and privacy-policy URLs against the production backend.

## Google Play Data safety

- Reconcile Data safety answers with the same production inventory.
- Declare coarse location only; fine location must not appear in the merged release manifest.
- Treat user-triggered transfer to a paired, user-owned host separately from developer collection, consistent with current Play guidance.
- Verify encrypted-in-transit answers against both HTTPS and Nola's application-layer encryption design.
- Review backup rules so credentials and private databases are not included in cloud/device-transfer backups.

## Release evidence

- Archived iOS privacy report.
- Merged Android release manifest and Data safety worksheet.
- Real-device permission screenshots for every capability.
- Network capture proving mobile payload ciphertext on allowed HTTP LAN connections.
- Negative tests proving public HTTP, host identity changes, certificate pin mismatches, replayed nonces, and unapproved Hybrid egress are rejected.
