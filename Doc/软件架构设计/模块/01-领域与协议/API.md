# Current Nola API

Reviewed against source on 2026-10-28. The active API has three separate boundaries:

| Boundary | Source / purpose | Reference |
| --- | --- | --- |
| Rust Host HTTP | `cli/src/web_parts/routes.rs`; local Web, paired clients, model management and Run observation | [Runtime contracts](CURRENT_RUNTIME_INTERFACES.md), [route catalog](INTERFACE_CATALOG.md#rust-host-固定路由) |
| Core Application | `ApplicationCommand`, called through the existing Service or embedded port | [Command tags and types](INTERFACE_CATALOG.md#application-顶层命令-tag), [Application port](../03-Application与事务/APPLICATION_PORT_CONTRACT.md) |
| Cloudflare Worker | `backend/worker/src/index.ts`; identity, channels, model proxy, sources/results, notifications and replicas | [Current dispatch rules](INTERFACE_CATALOG.md#worker-路由) |

Paired routes use their existing Bearer authentication and encrypted JSON envelope. Local Web sessions, Worker accounts, webhook signatures and admin credentials are separate authentication scopes; consult each linked handler. A successful transport response is not proof of a configuration commit, completed inference or stopped external operation.

A2UI v0.9.1, Service protocol, native C ABI, HTTP v2 and DTO protocol versions are independent. Preserve exact wire keys, original identities, CAS conditions and record digests. Stream disconnection detaches observation; explicit business cancellation requires its durable command and canonical confirmation. Unknown outcomes must not be retried with a new operation identity.

The catalog is extracted from actual source declarations and points to request/response types. It does not claim full end-to-end coverage or enumerate dynamic Rust paths as fixed routes. See the [Chinese API guide](API.zh-CN.md) for the detailed contract index. The superseded Worker list is retained only as a [dated historical reference](../../../历史任务/参考研究/API接口基线_20260719/API.md).
