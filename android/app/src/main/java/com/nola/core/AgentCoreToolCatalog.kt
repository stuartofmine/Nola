package com.nola.core

import kotlinx.serialization.json.JsonElement

object AgentCoreToolCatalog {
    /** A caller without live broker and executor state cannot expose any tool. */
    fun tools(executionMode: String): List<JsonElement> = emptyList()

    fun plan(
        executionMode: AndroidAgentExecutionMode,
        permissionMode: AndroidAgentPermissionMode = AndroidAgentPermissionMode.Cautious,
        permissionBroker: AndroidAgentPermissionBroker,
        executor: AndroidAgentHostExecutor,
        identity: AgentCoreRuntimeIdentity,
        resultEgressAuthorized: Boolean,
        uiToolSchema: JsonElement
    ): AgentCoreAndroidToolPlan {
        val snapshot = permissionBroker.runtimePolicySnapshot(permissionMode)
        return AndroidAgentToolPolicyPlanner.plan(
            executionMode = executionMode,
            permissionMode = permissionMode,
            snapshot = snapshot,
            uiToolSchema = uiToolSchema,
            handlerCapabilities = executor.runtimeHandlerCapabilities(),
            handlerIds = executor.runtimeHandlerIds(),
            identity = identity,
            resultEgressAuthorization = resultEgressAuthorization(
                identity,
                snapshot.capturedAtMs,
                resultEgressAuthorized
            )
        )
    }

    fun hybridAdvertisementTools(
        availableCapabilities: Set<String>,
        identity: AgentCoreRuntimeIdentity,
        resultEgressAuthorized: Boolean,
        permissionMode: AndroidAgentPermissionMode = AndroidAgentPermissionMode.Cautious,
    ): List<JsonElement> {
        val capabilities = availableCapabilities.intersect(AndroidAgentToolRegistry.capabilities)
        if (capabilities.isEmpty()) return emptyList()
        val capturedAtMs = System.currentTimeMillis()
        val snapshot = AndroidAgentPermissionSnapshot(
            capturedAtMs = capturedAtMs,
            authorizations = capabilities.associateWith { capability ->
                if (capability == "clipboard" && permissionMode != AndroidAgentPermissionMode.Cautious) {
                    "granted"
                } else {
                    "ask"
                }
            },
            availableCapabilities = capabilities,
            isForeground = true,
            scopedGrants = emptyList()
        )
        return AndroidAgentToolPolicyPlanner.plan(
            executionMode = AndroidAgentExecutionMode.Hybrid,
            permissionMode = permissionMode,
            snapshot = snapshot,
            handlerCapabilities = capabilities,
            handlerIds = AndroidAgentToolRegistry.handlerIds,
            identity = identity,
            resultEgressAuthorization = resultEgressAuthorization(
                identity,
                capturedAtMs,
                resultEgressAuthorized
            )
        ).tools
    }

    private fun resultEgressAuthorization(
        identity: AgentCoreRuntimeIdentity,
        issuedAtMs: Long,
        authorized: Boolean
    ): AndroidAgentResultEgressAuthorization? = if (authorized) {
        AndroidAgentResultEgressAuthorization(
            route = "device",
            identity = identity,
            issuedAtMs = issuedAtMs,
            expiresAtMs = issuedAtMs + 60_000L
        )
    } else {
        null
    }

}
