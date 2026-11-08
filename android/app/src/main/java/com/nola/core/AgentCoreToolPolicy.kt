@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.core

import java.util.UUID
import java.util.concurrent.atomic.AtomicLong
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

enum class AndroidAgentExecutionMode(
    val wireValue: String,
    internal val routes: Set<String>,
    val supportsDeviceExecution: Boolean
) {
    DeviceLocal("device_local", setOf("device"), true),
    HostRemote("host_remote", setOf("agent_host"), false),
    Hybrid("hybrid", setOf("device", "agent_host"), true);

    companion object {
        fun fromWireValue(value: String): AndroidAgentExecutionMode? = entries.firstOrNull {
            it.wireValue == value
        }
    }
}

@Serializable
data class AgentCoreRuntimeToolManifest(
    @SerialName("tool_id") val toolId: String,
    @SerialName("handler_id") val handlerId: String? = null,
    val schema: JsonElement,
    val route: String? = null,
    val available: Boolean = false
)

@Serializable
data class AgentCoreRuntimeResultEgressReceipt(
    @SerialName("tool_id") val toolId: String,
    val route: String,
    @SerialName("manifest_id") val manifestId: String,
    @SerialName("host_id") val hostId: String,
    @SerialName("client_id") val clientId: String,
    @SerialName("session_id") val sessionId: String,
    @SerialName("issued_at_ms") val issuedAtMs: Long,
    @SerialName("expires_at_ms") val expiresAtMs: Long,
    @SerialName("allow_result_egress") val allowResultEgress: Boolean
)

@Serializable
data class AgentCoreRuntimeCapabilityManifest(
    @SerialName("manifest_id") val manifestId: String,
    @SerialName("manifest_version") val manifestVersion: Long,
    @SerialName("issued_at_ms") val issuedAtMs: Long,
    @SerialName("expires_at_ms") val expiresAtMs: Long,
    @SerialName("host_id") val hostId: String? = null,
    @SerialName("client_id") val clientId: String? = null,
    @SerialName("session_id") val sessionId: String? = null,
    val routes: List<String>,
    val capabilities: List<String>,
    val authorizations: Map<String, String>,
    @SerialName("result_egress_receipts")
    val resultEgressReceipts: List<AgentCoreRuntimeResultEgressReceipt>,
    val tools: List<AgentCoreRuntimeToolManifest>
)

@Serializable
data class AgentCoreRuntimeIdentity(
    @SerialName("host_id") val hostId: String? = null,
    @SerialName("client_id") val clientId: String? = null,
    @SerialName("session_id") val sessionId: String? = null
)

@Serializable
data class AgentCoreScopedCapabilityGrant(
    val capability: String,
    val route: String? = null,
    val scope: String? = null,
    @SerialName("expires_at_ms") val expiresAtMs: Long
)

@Serializable
data class AgentCoreToolPolicyConversation(
    val mode: String,
    @SerialName("allow_tools") val allowTools: List<String> = emptyList(),
    @SerialName("deny_tools") val denyTools: List<String> = emptyList()
)

@Serializable
data class AgentCoreToolPolicyUserPolicy(
    @SerialName("allow_tools") val allowTools: List<String> = emptyList(),
    @SerialName("deny_tools") val denyTools: List<String> = emptyList(),
    @SerialName("allow_capabilities") val allowCapabilities: List<String> = emptyList(),
    @SerialName("deny_capabilities") val denyCapabilities: List<String> = emptyList(),
    @SerialName("max_risk") val maxRisk: String = "high",
    @SerialName("allow_result_egress_routes")
    val allowResultEgressRoutes: List<String> = emptyList()
)

@Serializable
data class AgentCoreResultEgressGrant(
    @SerialName("tool_id") val toolId: String,
    val route: String,
    @SerialName("manifest_id") val manifestId: String,
    @SerialName("host_id") val hostId: String,
    @SerialName("client_id") val clientId: String,
    @SerialName("session_id") val sessionId: String,
    @SerialName("expires_at_ms") val expiresAtMs: Long
)

@Serializable
data class AgentCoreToolPolicyInput(
    @SerialName("schema_version") val schemaVersion: Int = 2,
    val platform: String = "android",
    @SerialName("now_ms") val nowMs: Long,
    @SerialName("expected_identity") val expectedIdentity: AgentCoreRuntimeIdentity,
    val runtime: AgentCoreRuntimeCapabilityManifest,
    val conversation: AgentCoreToolPolicyConversation,
    @SerialName("user_policy") val userPolicy: AgentCoreToolPolicyUserPolicy,
    @SerialName("scoped_grants") val scopedGrants: List<AgentCoreScopedCapabilityGrant> = emptyList(),
    @SerialName("result_egress_grants")
    val resultEgressGrants: List<AgentCoreResultEgressGrant> = emptyList()
)

@Serializable
data class AgentCoreResolvedTool(
    @SerialName("tool_id") val toolId: String,
    val name: String,
    val enabled: Boolean,
    val route: String? = null,
    val capability: String,
    val permission: String,
    val authorization: String,
    val risk: String,
    @SerialName("requires_result_egress") val requiresResultEgress: Boolean,
    val reason: String,
    val schema: JsonElement
)

@Serializable
data class AgentCoreToolPolicyOutput(
    @SerialName("schema_version") val schemaVersion: Int,
    val platform: String,
    @SerialName("policy_digest") val policyDigest: String,
    @SerialName("manifest_digest") val manifestDigest: String,
    @SerialName("manifest_valid_until_ms") val manifestValidUntilMs: Long,
    @SerialName("enabled_tools") val enabledTools: List<AgentCoreResolvedTool> = emptyList(),
    @SerialName("disabled_tools") val disabledTools: List<AgentCoreResolvedTool> = emptyList()
)

data class AgentCoreAllowedAndroidTool(
    val toolId: String,
    val handlerId: String,
    val capability: String,
    val route: String,
    val permission: String,
    val authorization: String,
    val risk: String,
    val requiresResultEgress: Boolean
)

data class AndroidAgentScopedGrantFact(
    val capability: String,
    val route: String,
    val scope: String,
    val expiresAtMs: Long
)

data class AndroidAgentPermissionSnapshot(
    val capturedAtMs: Long,
    val authorizations: Map<String, String>,
    val availableCapabilities: Set<String>,
    val isForeground: Boolean,
    val scopedGrants: List<AndroidAgentScopedGrantFact>
)

data class AndroidAgentResultEgressAuthorization(
    val route: String,
    val identity: AgentCoreRuntimeIdentity,
    val issuedAtMs: Long,
    val expiresAtMs: Long
)

private const val RESULT_EGRESS_MAX_TTL_MS = 300_000L
private const val RESULT_EGRESS_CLOCK_SKEW_MS = 30_000L

internal fun AgentCoreToolPolicyInput.hasValidResultEgressAuthorization(
    toolId: String,
    route: String
): Boolean {
    val hostId = runtime.hostId?.trim().orEmpty()
    val clientId = runtime.clientId?.trim().orEmpty()
    val sessionId = runtime.sessionId?.trim().orEmpty()
    if (hostId.isEmpty() || clientId.isEmpty() || sessionId.isEmpty()) return false
    if (expectedIdentity.hostId?.trim()?.let { it != hostId } == true ||
        expectedIdentity.clientId?.trim()?.let { it != clientId } == true ||
        expectedIdentity.sessionId?.trim()?.let { it != sessionId } == true
    ) return false
    val receipt = runtime.resultEgressReceipts.firstOrNull {
        it.toolId.trim() == toolId && it.route == route &&
            it.manifestId == runtime.manifestId &&
            it.hostId.trim() == hostId && it.clientId.trim() == clientId &&
            it.sessionId.trim() == sessionId && it.allowResultEgress &&
            it.issuedAtMs > 0 && it.expiresAtMs > it.issuedAtMs &&
            it.expiresAtMs - it.issuedAtMs <= RESULT_EGRESS_MAX_TTL_MS &&
            it.issuedAtMs <= nowMs + RESULT_EGRESS_CLOCK_SKEW_MS &&
            it.expiresAtMs > nowMs && it.expiresAtMs <= runtime.expiresAtMs
    } ?: return false
    if (route !in userPolicy.allowResultEgressRoutes) return false
    return resultEgressGrants.any {
        it.toolId.trim() == toolId && it.route == route &&
            it.manifestId == runtime.manifestId &&
            it.hostId.trim() == hostId && it.clientId.trim() == clientId &&
            it.sessionId.trim() == sessionId &&
            it.expiresAtMs > nowMs && it.expiresAtMs <= receipt.expiresAtMs
    }
}

data class AgentCoreAndroidToolPlan(
    val tools: List<JsonElement>,
    val grantedCapabilities: List<String>,
    val allowedTools: List<AgentCoreAllowedAndroidTool> = emptyList(),
    val scopedGrants: List<AndroidAgentScopedGrantFact> = emptyList(),
    val policyDigest: String = "",
    val manifestDigest: String = "",
    val manifestValidUntilMs: Long = 0,
    val manifestId: String = "",
    val manifestVersion: Long = 0,
    val identity: AgentCoreRuntimeIdentity = AgentCoreRuntimeIdentity(),
    val planId: String = policyDigest,
    val originalIntent: AgentCoreToolPolicyInput? = null,
    val permissionMode: AndroidAgentPermissionMode = AndroidAgentPermissionMode.Cautious,
) {
    fun receiptIsValid(nowMs: Long = System.currentTimeMillis()): Boolean =
        policyDigest.isNotBlank() && manifestDigest.isNotBlank() &&
            manifestId.isNotBlank() && manifestVersion > 0 && manifestValidUntilMs > nowMs

    fun allows(toolName: String, capability: String, route: String = "device"): Boolean =
        receiptIsValid() && allowedTools.any {
            it.toolId == toolName && it.capability == capability && it.route == route
        }

    fun allowsPermission(toolName: String, capability: String, permission: String): Boolean =
        receiptIsValid() && allowedTools.any {
            it.toolId == toolName && it.capability == capability && it.permission == permission
        }

    fun riskFor(toolName: String, capability: String): String? =
        allowedTools.firstOrNull {
            it.toolId == toolName && it.capability == capability
        }?.risk

    companion object {
        fun empty(): AgentCoreAndroidToolPlan = AgentCoreAndroidToolPlan(emptyList(), emptyList())
    }
}

internal fun interface AgentCoreToolPolicyResolver {
    fun resolve(input: AgentCoreToolPolicyInput): AgentCoreToolPolicyOutput?
}

internal object AgentCoreToolPolicyCodec {
    private val json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
        encodeDefaults = true
    }

    fun encode(input: AgentCoreToolPolicyInput): ByteArray =
        json.encodeToString(input).encodeToByteArray()

    fun decode(bytes: ByteArray): AgentCoreToolPolicyOutput? = runCatching {
        json.decodeFromString(AgentCoreToolPolicyOutput.serializer(), bytes.decodeToString())
    }.getOrNull()
}

internal data class AndroidAgentToolCandidate(
    val toolId: String,
    val handlerId: String,
    val schema: JsonElement
) {
    fun manifest(route: String, available: Boolean): AgentCoreRuntimeToolManifest =
        AgentCoreRuntimeToolManifest(
            toolId = toolId,
            handlerId = handlerId,
            schema = schema,
            route = route,
            available = available
        )
}

/** Canonical Android handler facts. Policy metadata is intentionally absent. */
internal object AndroidAgentToolRegistry {
    private val handlers = linkedMapOf(
        "device_read_file" to "android.files.read",
        "device_write_file" to "android.files.write",
        "device_clipboard_read" to "android.clipboard.read",
        "device_clipboard_write" to "android.clipboard.write",
        "device_get_location" to "android.location.current",
        "device_search_calendar_events" to "android.calendar.search",
        "device_create_calendar_event" to "android.calendar.create",
        "memory_search" to "android.memory.search",
        "skill_invoke" to "android.skills.invoke",
        "render_ui" to "android.core.ui.publish"
    )

    val capabilities: Set<String> = setOf(
        "files_read",
        "files_write",
        "clipboard",
        "location",
        "calendar",
        "none",
        "skill_context"
    )
    val handlerIds: Set<String> = handlers.values.toSet()

    val candidates: List<AndroidAgentToolCandidate> = listOf(
        candidate("device_read_file", "Read a UTF-8 text file selected for this app.", "ordinary_read", mapOf("path" to string("File path or content URI.")), listOf("path")),
        candidate("device_write_file", "Write UTF-8 text to a file selected for this app.", "unknown", mapOf("path" to string("File path or content URI."), "content" to string("Complete UTF-8 file content.")), listOf("path", "content")),
        candidate("device_clipboard_read", "Read text from the Android clipboard.", "sensitive_read"),
        candidate("device_clipboard_write", "Replace the Android clipboard text.", "overwrite", mapOf("text" to string("Text to place on the clipboard.")), listOf("text")),
        candidate("device_get_location", "Read the device's current approximate location.", "sensitive_read"),
        candidate("device_search_calendar_events", "Search calendar events in a time range.", "sensitive_read", mapOf("start_ms" to integer("Inclusive Unix time in milliseconds."), "end_ms" to integer("Exclusive Unix time in milliseconds."))),
        candidate("device_create_calendar_event", "Create an event in the Android calendar.", "new_file", mapOf("title" to string("Event title."), "start_ms" to integer("Start Unix time in milliseconds."), "end_ms" to integer("End Unix time in milliseconds."), "notes" to string("Optional event notes.")), listOf("title", "start_ms", "end_ms")),
        candidate(
            "memory_search",
            "Search durable long-term memory when earlier user context may help the current task. The query is chosen by the model and is not derived by the client.",
            "ordinary_read",
            mapOf(
                "query" to string("Search query describing the memory evidence needed."),
                "limit" to integer("Maximum number of memory records to return, from 1 to 20."),
            ),
            listOf("query"),
        ),
        candidate(
            "skill_invoke",
            "Load one enabled Skill by its exact advertised name. The complete instructions become active only after this call succeeds.",
            "ordinary_read",
            mapOf(
                "skill" to string("Exact enabled Skill name from the Skill catalog."),
                "args" to buildJsonObject {
                    put("type", "object")
                    put("description", "Optional structured arguments for the Skill.")
                    put("additionalProperties", true)
                },
                "activation" to string("Activation scope: turn or session."),
            ),
            listOf("skill"),
        )
    )

    fun canHandle(toolId: String, handlerId: String): Boolean = handlers[toolId] == handlerId
    fun handlerId(toolId: String): String? = handlers[toolId]

    private fun candidate(
        toolId: String,
        description: String,
        permissionRisk: String,
        properties: Map<String, JsonElement> = emptyMap(),
        required: List<String> = emptyList()
    ): AndroidAgentToolCandidate {
        val handlerId = checkNotNull(handlers[toolId]) { "Tool $toolId has no Android handler" }
        val schema = buildJsonObject {
            put("type", "function")
            put("x_nola", buildJsonObject {
                put("permission_contract", buildJsonObject { put("risk", permissionRisk) })
            })
            put("function", buildJsonObject {
                put("name", toolId)
                put("description", description)
                put("parameters", buildJsonObject {
                    put("type", "object")
                    put("properties", JsonObject(properties))
                    put("additionalProperties", false)
                    if (required.isNotEmpty()) {
                        put("required", JsonArray(required.map { JsonPrimitive(it) }))
                    }
                })
            })
        }
        return AndroidAgentToolCandidate(toolId, handlerId, schema)
    }

    private fun string(description: String): JsonElement = buildJsonObject {
        put("type", "string")
        put("description", description)
    }

    private fun integer(description: String): JsonElement = buildJsonObject {
        put("type", "integer")
        put("description", description)
    }
}

internal object AndroidAgentToolPolicyPlanner {
    private const val MANIFEST_TTL_MS = 60_000L
    private val manifestId = "android-${UUID.randomUUID()}"
    private val manifestVersion = AtomicLong(0)
    private val nativeResolver = AgentCoreToolPolicyResolver { input ->
        AgentCoreToolPolicyEvaluator.evaluate(input)
    }

    internal fun plan(
        executionMode: AndroidAgentExecutionMode,
        permissionMode: AndroidAgentPermissionMode = AndroidAgentPermissionMode.Cautious,
        snapshot: AndroidAgentPermissionSnapshot,
        handlerCapabilities: Set<String>,
        handlerIds: Set<String>,
        identity: AgentCoreRuntimeIdentity = AgentCoreRuntimeIdentity(),
        resultEgressAuthorization: AndroidAgentResultEgressAuthorization?,
        resolver: AgentCoreToolPolicyResolver = nativeResolver,
        uiToolSchema: JsonElement? = null
    ): AgentCoreAndroidToolPlan {
        val nowMs = maxOf(snapshot.capturedAtMs, System.currentTimeMillis())
        val issuedAtMs = nowMs
        val expiresAtMs = nowMs + MANIFEST_TTL_MS
        val version = manifestVersion.incrementAndGet()
        val capabilities = snapshot.availableCapabilities.intersect(handlerCapabilities).sorted()
        val route = if (executionMode.supportsDeviceExecution) "device" else "agent_host"
        val candidates = AndroidAgentToolRegistry.candidates + listOfNotNull(uiToolSchema?.let {
            AndroidAgentToolCandidate("render_ui", checkNotNull(AndroidAgentToolRegistry.handlerId("render_ui")), it)
        })
        val manifests = candidates.map { candidate ->
            candidate.manifest(
                route = "device",
                available = executionMode.supportsDeviceExecution && snapshot.isForeground &&
                    candidate.handlerId in handlerIds &&
                    AndroidAgentToolRegistry.canHandle(candidate.toolId, candidate.handlerId)
            )
        }
        val egressAuthorization = resultEgressAuthorization?.takeIf {
            it.route == "device" && it.identity == identity &&
                !identity.hostId.isNullOrBlank() && !identity.clientId.isNullOrBlank() &&
                !identity.sessionId.isNullOrBlank() && it.issuedAtMs > 0 &&
                it.issuedAtMs <= nowMs + RESULT_EGRESS_CLOCK_SKEW_MS &&
                it.expiresAtMs > nowMs && it.expiresAtMs > it.issuedAtMs &&
                it.expiresAtMs - it.issuedAtMs <= RESULT_EGRESS_MAX_TTL_MS
        }
        val egressExpiresAtMs = minOf(egressAuthorization?.expiresAtMs ?: 0, expiresAtMs)
        val resultEgressReceipts = egressAuthorization?.let { authorization ->
            manifests.filter { it.available && it.route == authorization.route }.map { tool ->
                AgentCoreRuntimeResultEgressReceipt(
                    toolId = tool.toolId,
                    route = authorization.route,
                    manifestId = manifestId,
                    hostId = checkNotNull(identity.hostId),
                    clientId = checkNotNull(identity.clientId),
                    sessionId = checkNotNull(identity.sessionId),
                    issuedAtMs = authorization.issuedAtMs,
                    expiresAtMs = egressExpiresAtMs,
                    allowResultEgress = true
                )
            }
        }.orEmpty()
        val resultEgressGrants = resultEgressReceipts.map { receipt ->
            AgentCoreResultEgressGrant(
                toolId = receipt.toolId,
                route = receipt.route,
                manifestId = receipt.manifestId,
                hostId = receipt.hostId,
                clientId = receipt.clientId,
                sessionId = receipt.sessionId,
                expiresAtMs = receipt.expiresAtMs
            )
        }
        val scopedGrants = snapshot.scopedGrants.filter { it.expiresAtMs > nowMs }.map {
            AgentCoreScopedCapabilityGrant(
                capability = it.capability,
                route = it.route,
                scope = it.scope,
                expiresAtMs = it.expiresAtMs
            )
        }
        val runtime = AgentCoreRuntimeCapabilityManifest(
            manifestId = manifestId,
            manifestVersion = version,
            issuedAtMs = issuedAtMs,
            expiresAtMs = expiresAtMs,
            hostId = identity.hostId,
            clientId = identity.clientId,
            sessionId = identity.sessionId,
            routes = executionMode.routes.sorted(),
            capabilities = capabilities,
            authorizations = snapshot.authorizations.filterKeys(capabilities::contains),
            resultEgressReceipts = resultEgressReceipts,
            tools = manifests
        )
        val input = AgentCoreToolPolicyInput(
            nowMs = nowMs,
            expectedIdentity = identity,
            runtime = runtime,
            conversation = AgentCoreToolPolicyConversation(executionMode.wireValue),
            userPolicy = AgentCoreToolPolicyUserPolicy(
                maxRisk = "high",
                allowResultEgressRoutes = if (resultEgressReceipts.isEmpty()) {
                    emptyList()
                } else {
                    listOf("device")
                }
            ),
            scopedGrants = scopedGrants,
            resultEgressGrants = resultEgressGrants
        )
        val output = runCatching { resolver.resolve(input) }.getOrNull()
            ?: return AgentCoreAndroidToolPlan.empty()
        if (output.schemaVersion != 2 || output.platform != "android" ||
            output.policyDigest.isBlank() || output.manifestDigest.isBlank() ||
            output.manifestValidUntilMs != expiresAtMs || output.manifestValidUntilMs <= nowMs
        ) {
            return AgentCoreAndroidToolPlan.empty()
        }

        val enabled = output.enabledTools.mapNotNull { resolved ->
            val candidate = candidates.firstOrNull {
                it.toolId == resolved.toolId
            } ?: return@mapNotNull null
            val advertised = runtime.tools.firstOrNull { it.toolId == resolved.toolId }
                ?: return@mapNotNull null
            if (!resolved.enabled || resolved.name != candidate.toolId ||
                resolved.route != "device" || resolved.schema != candidate.schema ||
                resolved.authorization !in setOf("ask", "granted") ||
                resolved.capability !in capabilities ||
                !advertised.available || advertised.handlerId != candidate.handlerId ||
                !AndroidAgentToolRegistry.canHandle(candidate.toolId, candidate.handlerId) ||
                resolved.route == "device" && resolved.requiresResultEgress &&
                    !input.hasValidResultEgressAuthorization(resolved.toolId, "device")
            ) return@mapNotNull null
            candidate to resolved
        }.distinctBy { it.first.toolId }

        val allowedTools = enabled.map { (candidate, resolved) ->
            AgentCoreAllowedAndroidTool(
                toolId = candidate.toolId,
                handlerId = candidate.handlerId,
                capability = resolved.capability,
                route = resolved.route.orEmpty(),
                permission = resolved.permission,
                authorization = resolved.authorization,
                risk = resolved.risk,
                requiresResultEgress = resolved.requiresResultEgress
            )
        }
        return AgentCoreAndroidToolPlan(
            tools = enabled.map { (_, resolved) -> schemaWithCoreMetadata(resolved, output) },
            grantedCapabilities = allowedTools.filter { it.authorization == "granted" }
                .map { it.capability }.distinct().sorted(),
            allowedTools = allowedTools,
            scopedGrants = snapshot.scopedGrants.filter { it.expiresAtMs > nowMs },
            policyDigest = output.policyDigest,
            manifestDigest = output.manifestDigest,
            manifestValidUntilMs = output.manifestValidUntilMs,
            manifestId = manifestId,
            manifestVersion = version,
            identity = identity,
            planId = output.policyDigest,
            originalIntent = input,
            permissionMode = permissionMode,
        )
    }

    private fun schemaWithCoreMetadata(
        resolved: AgentCoreResolvedTool,
        output: AgentCoreToolPolicyOutput
    ): JsonElement {
        val root = resolved.schema as? JsonObject ?: return resolved.schema
        val metadata = buildJsonObject {
            (root["x_nola"] as? JsonObject)?.get("permission_contract")?.let { put("permission_contract", it) }
            put("target", resolved.route ?: "device")
            put("capability", resolved.capability)
            put("authorization", resolved.permission)
            put("authorization_status", resolved.authorization)
            put("risk", resolved.risk)
            put("requires_result_egress", resolved.requiresResultEgress)
            put("tool_id", resolved.toolId)
            put("policy_digest", output.policyDigest)
            put("manifest_digest", output.manifestDigest)
            put("manifest_valid_until_ms", output.manifestValidUntilMs)
        }
        return JsonObject(root + ("x_nola" to metadata))
    }
}

/** Renew observation receipts only; the Run's admitted tools and grants cannot change. */
internal fun revalidateFrozenAndroidToolPlan(
    frozen: AgentCoreAndroidToolPlan, fresh: AgentCoreAndroidToolPlan, nowMs: Long = System.currentTimeMillis(),
): AgentCoreAndroidToolPlan {
    fun schemas(plan: AgentCoreAndroidToolPlan) = plan.tools.map { element ->
        val root = element.jsonObject
        val metadata = root["x_nola"]?.jsonObject.orEmpty() - setOf("policy_digest", "manifest_digest", "manifest_valid_until_ms")
        JsonObject(root + ("x_nola" to JsonObject(metadata)))
    }
    fun grants(plan: AgentCoreAndroidToolPlan) = plan.scopedGrants.map { Triple(it.capability, it.route, it.scope) }.toSet()
    require(fresh.receiptIsValid(nowMs) && frozen.identity == fresh.identity &&
        frozen.permissionMode == fresh.permissionMode && frozen.allowedTools == fresh.allowedTools &&
        frozen.grantedCapabilities.toSet() == fresh.grantedCapabilities.toSet() &&
        schemas(frozen) == schemas(fresh) && grants(frozen) == grants(fresh)) {
        "Android tool authority changed; the frozen Run cannot adopt new permissions"
    }
    return fresh
}

/** Shared by foreground and independent child dispatch before permission/tool execution. */
internal fun androidToolPlanForHostRequest(
    requestType: String?, frozen: AgentCoreAndroidToolPlan,
    refresh: (() -> AgentCoreAndroidToolPlan)?, nowMs: Long = System.currentTimeMillis(),
): AgentCoreAndroidToolPlan {
    if (requestType !in setOf("request_permission", "execute_tool") || frozen.receiptIsValid(nowMs)) return frozen
    return revalidateFrozenAndroidToolPlan(frozen,
        checkNotNull(refresh) { "Expired tool authority cannot be refreshed" }(), nowMs)
}
