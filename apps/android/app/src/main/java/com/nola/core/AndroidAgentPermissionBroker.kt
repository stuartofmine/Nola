package com.nola.core

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.core.content.ContextCompat
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive
import java.util.concurrent.ConcurrentHashMap

data class AndroidAgentPermissionRequest(
    val allowConversation: Boolean,
    val requestId: String,
    val callId: String,
    val toolName: String,
    val capability: String,
    val authorization: String,
    val rationale: String,
    val scope: String?,
    val permissionInstructions: List<AgentPermissionInstruction> = emptyList(),
    val allowsInstruction: Boolean = true,
    val originKind: String? = null,
)

sealed interface AndroidAgentPermissionAction {
    val requestId: String

    data class Immediate(
        override val requestId: String,
        val allow: Boolean
    ) : AndroidAgentPermissionAction

    data class RuntimePermissions(
        override val requestId: String,
        val permissions: List<String>
    ) : AndroidAgentPermissionAction

    data class OpenDocument(
        override val requestId: String,
        val write: Boolean,
        val mimeTypes: List<String> = listOf("*/*")
    ) : AndroidAgentPermissionAction

    data class PickVisualMedia(override val requestId: String) : AndroidAgentPermissionAction
    data class HealthConnect(override val requestId: String) : AndroidAgentPermissionAction
    data class Unavailable(
        override val requestId: String,
        val reason: String
    ) : AndroidAgentPermissionAction
}

enum class AndroidAgentPermissionUserDecision(val wireValue: String) {
    AllowOnce("allow_once"),
    AllowConversation("allow_conversation"),
    Deny("deny"),
    ProvideInstruction("provide_instruction")
}

data class AndroidAgentPermissionDecision(
    val requestId: String,
    val decision: AndroidAgentPermissionUserDecision,
    val scopedUri: String? = null,
    val instruction: String? = null,
) {
    val isAllowed: Boolean
        get() = decision == AndroidAgentPermissionUserDecision.AllowOnce || decision == AndroidAgentPermissionUserDecision.AllowConversation

    fun withConversationScope(enabled: Boolean): AndroidAgentPermissionDecision = copy(
        decision = if (enabled && isAllowed) {
            AndroidAgentPermissionUserDecision.AllowConversation
        } else {
            decision
        }
    )
}

class AndroidAgentPermissionBroker(private val context: Context) {
    private data class BoundPlan(val plan: AgentCoreAndroidToolPlan, val refresh: (() -> AgentCoreAndroidToolPlan)?)
    private val requestPlans = ConcurrentHashMap<String, BoundPlan>()
    private val coreConfirmations = ConcurrentHashMap<String, AndroidAgentPermissionRequest>()

    /** Only called after Core validates the started request and its domain-free descriptor. */
    internal fun bindCoreConfirmation(request: AndroidAgentPermissionRequest) {
        require(request.capability == "none" && request.authorization == "none")
        coreConfirmations[request.requestId] = request
    }

    fun bindPermissionRequest(
        request: AndroidAgentPermissionRequest,
        plan: AgentCoreAndroidToolPlan,
        refresh: (() -> AgentCoreAndroidToolPlan)? = null,
    ): Boolean {
        if (!plan.receiptIsValid() || !plan.allowsPermission(
                request.toolName,
                request.capability,
                request.authorization,
            )
        ) return false
        requestPlans[request.requestId] = BoundPlan(plan, refresh)
        return true
    }

    fun clearPermissionRequest(requestId: String) {
        requestPlans.remove(requestId)
        coreConfirmations.remove(requestId)
    }

    fun runtimeIdentity(sessionId: String): AgentCoreRuntimeIdentity {
        val deviceId = android.provider.Settings.Secure.getString(
            context.contentResolver,
            android.provider.Settings.Secure.ANDROID_ID
        )?.trim().orEmpty()
        return AgentCoreRuntimeIdentity(
            hostId = deviceId.takeIf(String::isNotEmpty)?.let { "android-device:$it" },
            clientId = context.packageName.takeIf(String::isNotEmpty),
            sessionId = sessionId.trim().takeIf(String::isNotEmpty)
        )
    }

    fun runtimePolicySnapshot(
        permissionMode: AndroidAgentPermissionMode = AndroidAgentPermissionMode.Cautious,
    ): AndroidAgentPermissionSnapshot {
        val capturedAtMs = System.currentTimeMillis()
        val persisted = context.contentResolver.persistedUriPermissions
        val scopedGrants = buildList {
            persisted.forEach { permission ->
                val scope = permission.uri.toString()
                val expiresAtMs = capturedAtMs + 60_000
                if (permission.isReadPermission) {
                    add(AndroidAgentScopedGrantFact("files_read", "device", scope, expiresAtMs))
                }
                if (permission.isWritePermission) {
                    add(AndroidAgentScopedGrantFact("files_write", "device", scope, expiresAtMs))
                }
            }
        }
        val locationGranted = listOf(
            android.Manifest.permission.ACCESS_COARSE_LOCATION,
            android.Manifest.permission.ACCESS_FINE_LOCATION
        ).any { permission ->
            androidx.core.content.ContextCompat.checkSelfPermission(context, permission) ==
                android.content.pm.PackageManager.PERMISSION_GRANTED
        }
        val calendarGranted = listOf(
            android.Manifest.permission.READ_CALENDAR,
            android.Manifest.permission.WRITE_CALENDAR
        ).all { permission ->
            androidx.core.content.ContextCompat.checkSelfPermission(context, permission) ==
                android.content.pm.PackageManager.PERMISSION_GRANTED
        }
        val processInfo = android.app.ActivityManager.RunningAppProcessInfo()
        android.app.ActivityManager.getMyMemoryState(processInfo)
        val foreground = processInfo.importance <=
            android.app.ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND_SERVICE
        return AndroidAgentPermissionSnapshot(
            capturedAtMs = capturedAtMs,
            authorizations = mapOf(
                "none" to "granted",
                "skill_context" to "granted",
                "files_read" to "ask",
                "files_write" to "ask",
                "clipboard" to if (permissionMode == AndroidAgentPermissionMode.Cautious) "ask" else "granted",
                "location" to if (locationGranted) "granted" else "ask",
                "calendar" to if (calendarGranted) "granted" else "ask"
            ),
            availableCapabilities = AndroidAgentToolRegistry.capabilities,
            isForeground = foreground,
            scopedGrants = scopedGrants
        )
    }

    fun actionFor(
        request: AndroidAgentPermissionRequest,
        userApproved: Boolean
    ): AndroidAgentPermissionAction {
        val requestId = request.requestId
        val confirmation = coreConfirmations.remove(requestId)
        if (confirmation != null) {
            if (confirmation != request) return AndroidAgentPermissionAction.Unavailable(requestId,
                "The Core permission request changed")
            return AndroidAgentPermissionAction.Immediate(requestId, allow = userApproved)
        }
        val bound = requestPlans.remove(requestId)
        val plan = bound?.let {
            if (it.plan.receiptIsValid()) it.plan else runCatching {
                revalidateFrozenAndroidToolPlan(it.plan, checkNotNull(it.refresh)())
            }.getOrNull()
        }
        if (plan == null || !plan.receiptIsValid() || !plan.allowsPermission(
                request.toolName,
                request.capability,
                request.authorization,
            )
        ) {
            return AndroidAgentPermissionAction.Unavailable(
                requestId,
                "Permission request is not bound to a valid Core tool plan"
            )
        }
        if (!userApproved) return AndroidAgentPermissionAction.Immediate(requestId, allow = false)

        return when (request.capability) {
            "none", "clipboard" -> AndroidAgentPermissionAction.Immediate(requestId, allow = true)
            "files_read" -> AndroidAgentPermissionAction.OpenDocument(requestId, write = false)
            "files_write" -> AndroidAgentPermissionAction.OpenDocument(requestId, write = true)
            "location" -> runtimeAction(
                requestId,
                listOf(Manifest.permission.ACCESS_COARSE_LOCATION)
            )
            "calendar" -> calendarAction(request, plan)
            else -> AndroidAgentPermissionAction.Unavailable(requestId, "Unsupported Android host capability")
        }
    }

    fun runtimeResult(requestId: String, grants: Map<String, Boolean>): AndroidAgentPermissionDecision =
        AndroidAgentPermissionDecision(
            requestId = requestId,
            decision = if (grants.isNotEmpty() && grants.values.all { it }) {
                AndroidAgentPermissionUserDecision.AllowOnce
            } else {
                AndroidAgentPermissionUserDecision.Deny
            }
        )

    fun scopedResourceResult(
        requestId: String,
        uri: Uri?,
        write: Boolean,
        persistable: Boolean = false
    ): AndroidAgentPermissionDecision {
        if (uri == null) return AndroidAgentPermissionDecision(
            requestId,
            AndroidAgentPermissionUserDecision.Deny
        )
        if (persistable) {
            val flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or
                (if (write) Intent.FLAG_GRANT_WRITE_URI_PERMISSION else 0)
            runCatching { context.contentResolver.takePersistableUriPermission(uri, flags) }
                .getOrElse {
                    return AndroidAgentPermissionDecision(
                        requestId,
                        AndroidAgentPermissionUserDecision.Deny
                    )
                }
        }
        return AndroidAgentPermissionDecision(
            requestId = requestId,
            decision = AndroidAgentPermissionUserDecision.AllowOnce,
            scopedUri = uri.toString()
        )
    }

    private fun runtimeAction(
        requestId: String,
        permissions: List<String>
    ): AndroidAgentPermissionAction {
        val missing = permissions.filter {
            ContextCompat.checkSelfPermission(context, it) != PackageManager.PERMISSION_GRANTED
        }
        return if (missing.isEmpty()) {
            AndroidAgentPermissionAction.Immediate(requestId, allow = true)
        } else {
            AndroidAgentPermissionAction.RuntimePermissions(requestId, missing)
        }
    }

    private fun calendarAction(
        request: AndroidAgentPermissionRequest,
        plan: AgentCoreAndroidToolPlan,
    ): AndroidAgentPermissionAction {
        val writeOperation = plan.riskFor(request.toolName, request.capability) != "read_only"
        val permissions = if (writeOperation) {
            listOf(Manifest.permission.READ_CALENDAR, Manifest.permission.WRITE_CALENDAR)
        } else {
            listOf(Manifest.permission.READ_CALENDAR)
        }
        return runtimeAction(request.requestId, permissions)
    }
}
