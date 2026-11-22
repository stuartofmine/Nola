# MOD-07 · Permissions and platform capabilities

[简体中文](README.md) · English · [Architecture](../../README.en.md)

Core evaluates allow, ask or deny under the cautious or allow_all application policy. Application persists requests and decisions. Platform adapters obtain OS permission. Pairing, model selection and enabling an extension establish separate facts.

## Runtime behavior

Approval binds the concrete call, resource, scope and policy in force. Resolve an OS denial on the device performing the operation. After a policy change, reconcile the outstanding request against its identity and the new policy.

## Failure and verification

Test a denied permission, a revoked grant, an obsolete approval and a tool unavailable on the executing Host. Show the user which resource was affected and where they can act. A UI control cannot grant a system privilege.

## Implementation

- [agent/runtime/permission_policy.rs](../../../../core/nola-agent-engine/src/agent/runtime/permission_policy.rs)
- [nola-store/src/permission_source.rs](../../../../core/nola-store/src/permission_source.rs)
- [src/tools/platform_catalog_contract_tests.rs](../../../../apps/cli/src/tools/platform_catalog_contract_tests.rs)

## Detailed references

[Git Inspection Authority](GIT_INSPECTION_AUTHORITY.md) · [Language Service](LANGUAGE_SERVICE.md) · [Permission Boundary Contract](PERMISSION_BOUNDARY_CONTRACT.md) · [Permission Closure](PERMISSION_CLOSURE.md) · [Remote Host Tool Plan](REMOTE_HOST_TOOL_PLAN.md) · [Terminal Session](TERMINAL_SESSION.md)
